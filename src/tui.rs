//! Interactive terminal UI (ratatui): profile list + detail, an edit form, and
//! a doctor view, all driven through the shared `actions`/`doctor` modules.

use anyhow::Result;
use std::io::{self, IsTerminal, Stdout};

use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::actions;
use crate::doctor::{self, Finding};
use crate::git::Scope;
use crate::profile::{Profile, Registry, Signing, Ssh};
use crate::signers;
use crate::sshconfig;

type Term = Terminal<CrosstermBackend<Stdout>>;

// Editable fields, in display order (label, hint).
const FIELDS: &[(&str, &str)] = &[
    ("name", "slug, e.g. work"),
    ("user.name", "display name"),
    ("user.email", "commit email"),
    ("host", "credential host, e.g. https://github.com (optional)"),
    ("username", "account username for HTTPS (optional)"),
    ("remote-match", "glob for auto-switch, e.g. https://github.com/org/** (optional)"),
    ("sign-format", "ssh | openpgp (optional)"),
    ("sign-key", "GPG id or ~/.ssh/key.pub (optional)"),
    ("auto-sign", "press space to toggle"),
    ("ssh-key", "private key path, e.g. ~/.ssh/id_work (optional)"),
    ("ssh-host-alias", "alias in ~/.ssh/config, e.g. github.com-work (optional)"),
    ("ssh-hostname", "real host for the alias (default github.com)"),
];
const F_NAME: usize = 0;
const F_AUTOSIGN: usize = 8;
const F_SSH_KEY: usize = 9;
const F_SSH_ALIAS: usize = 10;
const F_SSH_HOST: usize = 11;

enum Screen {
    List,
    Edit,
    Doctor,
}

struct EditState {
    adding: bool,
    original_name: String,
    values: Vec<String>,
    focus: usize,
}

impl EditState {
    fn blank() -> Self {
        EditState {
            adding: true,
            original_name: String::new(),
            values: vec![String::new(); FIELDS.len()],
            focus: F_NAME,
        }
    }

    fn from_profile(p: &Profile) -> Self {
        let mut values = vec![String::new(); FIELDS.len()];
        values[F_NAME] = p.name.clone();
        values[1] = p.user_name.clone();
        values[2] = p.user_email.clone();
        values[3] = p.host.clone().unwrap_or_default();
        values[4] = p.username.clone().unwrap_or_default();
        values[5] = p.remote_match.clone().unwrap_or_default();
        if let Some(s) = &p.signing {
            values[6] = s.format.clone();
            values[7] = s.key.clone();
            values[F_AUTOSIGN] = s.auto_sign.to_string();
        } else {
            values[F_AUTOSIGN] = "false".to_string();
        }
        if let Some(ssh) = &p.ssh {
            values[F_SSH_KEY] = ssh.key.clone();
            values[F_SSH_ALIAS] = ssh.host_alias.clone().unwrap_or_default();
            values[F_SSH_HOST] = ssh.hostname.clone().unwrap_or_default();
        }
        EditState {
            adding: false,
            original_name: p.name.clone(),
            // Editing starts past the (read-only) name field.
            focus: 1,
            values,
        }
    }
}

struct Row {
    name: String,
    active: bool,
    auto: bool,
}

struct App {
    reg: Registry,
    rows: Vec<Row>,
    sel: usize,
    screen: Screen,
    status: String,
    edit: EditState,
    findings: Vec<Finding>,
    confirm: Option<Confirm>,
    should_quit: bool,
}

struct Confirm {
    prompt: String,
    action: ConfirmAction,
}

enum ConfirmAction {
    Delete(String),
    FixAll,
}

impl App {
    fn load() -> Result<Self> {
        let mut app = App {
            reg: Registry::load()?,
            rows: Vec::new(),
            sel: 0,
            screen: Screen::List,
            status: "↑↓ move · u use · l local · g default · a auto · n new · e edit · d delete · c doctor · q quit".to_string(),
            edit: EditState::blank(),
            findings: Vec::new(),
            confirm: None,
            should_quit: false,
        };
        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        self.reg = Registry::load()?;
        let mut rows = Vec::new();
        for p in &self.reg.profiles {
            rows.push(Row {
                name: p.name.clone(),
                active: actions::global_active(p).unwrap_or(false),
                auto: actions::auto_enabled(p).unwrap_or(false),
            });
        }
        self.rows = rows;
        if self.sel >= self.rows.len() {
            self.sel = self.rows.len().saturating_sub(1);
        }
        Ok(())
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.rows.get(self.sel).and_then(|r| self.reg.get(&r.name))
    }
}

pub fn launch() -> Result<()> {
    if !io::stdout().is_terminal() {
        anyhow::bail!("`gum tui` needs an interactive terminal (stdout is not a TTY)");
    }

    // Restore the terminal even if we panic mid-render.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run(terminal: &mut Term) -> Result<()> {
    let mut app = App::load()?;
    while !app.should_quit {
        terminal.draw(|f| draw(f, &app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            handle_key(&mut app, key.code)?;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    // A confirm overlay swallows input until answered.
    if app.confirm.is_some() {
        return handle_confirm(app, code);
    }
    match app.screen {
        Screen::List => handle_list(app, code),
        Screen::Edit => handle_edit(app, code),
        Screen::Doctor => handle_doctor(app, code),
    }
}

fn handle_confirm(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let action = app.confirm.take().unwrap().action;
            match action {
                ConfirmAction::Delete(name) => {
                    delete_profile(app, &name)?;
                }
                ConfirmAction::FixAll => {
                    fix_all(app)?;
                }
            }
        }
        _ => {
            app.confirm = None;
            app.status = "Cancelled.".to_string();
        }
    }
    Ok(())
}

fn handle_list(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => {
            if app.sel > 0 {
                app.sel -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.sel + 1 < app.rows.len() {
                app.sel += 1;
            }
        }
        KeyCode::Char('u') => apply_selected(app, Scope::Global),
        KeyCode::Char('l') => apply_selected(app, Scope::Local),
        KeyCode::Char('g') => set_default_selected(app),
        KeyCode::Char('a') => toggle_auto(app)?,
        KeyCode::Char('n') => {
            app.edit = EditState::blank();
            app.screen = Screen::Edit;
            app.status = "New profile — Tab/↑↓ move · Enter save · Esc cancel".to_string();
        }
        KeyCode::Char('e') => {
            if let Some(p) = app.selected_profile() {
                app.edit = EditState::from_profile(p);
                app.screen = Screen::Edit;
                app.status = "Editing — Tab/↑↓ move · Enter save · Esc cancel".to_string();
            }
        }
        KeyCode::Char('d') => {
            if let Some(r) = app.rows.get(app.sel) {
                app.confirm = Some(Confirm {
                    prompt: format!("Delete profile '{}'? (y/N)", r.name),
                    action: ConfirmAction::Delete(r.name.clone()),
                });
            }
        }
        KeyCode::Char('c') => {
            app.findings = doctor::audit()?;
            app.screen = Screen::Doctor;
            app.status = "Doctor — f fix all · Esc back".to_string();
        }
        KeyCode::Char('r') => {
            app.refresh()?;
            app.status = "Refreshed.".to_string();
        }
        _ => {}
    }
    Ok(())
}

fn handle_edit(app: &mut App, code: KeyCode) -> Result<()> {
    let min_focus = if app.edit.adding { 0 } else { 1 };
    match code {
        KeyCode::Esc => {
            app.screen = Screen::List;
            app.status = "Cancelled.".to_string();
        }
        KeyCode::Enter => save_edit(app)?,
        KeyCode::Tab | KeyCode::Down => {
            app.edit.focus = (app.edit.focus + 1).min(FIELDS.len() - 1);
        }
        KeyCode::BackTab | KeyCode::Up => {
            if app.edit.focus > min_focus {
                app.edit.focus -= 1;
            }
        }
        KeyCode::Char(' ') if app.edit.focus == F_AUTOSIGN => {
            let v = &mut app.edit.values[F_AUTOSIGN];
            *v = if v == "true" { "false".into() } else { "true".into() };
        }
        KeyCode::Char(c) if app.edit.focus != F_AUTOSIGN => {
            app.edit.values[app.edit.focus].push(c);
        }
        KeyCode::Backspace if app.edit.focus != F_AUTOSIGN => {
            app.edit.values[app.edit.focus].pop();
        }
        _ => {}
    }
    Ok(())
}

fn handle_doctor(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::List;
            app.refresh()?;
        }
        KeyCode::Char('f') => {
            let fixable = app.findings.iter().filter(|f| f.is_fixable()).count();
            if fixable == 0 {
                app.status = "Nothing to fix.".to_string();
            } else {
                app.confirm = Some(Confirm {
                    prompt: format!("Apply {fixable} fix(es) to ~/.gitconfig? (y/N)"),
                    action: ConfirmAction::FixAll,
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_selected(app: &mut App, scope: Scope) {
    let Some(p) = app.selected_profile() else {
        return;
    };
    let name = p.name.clone();
    match actions::apply_profile(p, scope) {
        Ok(()) => app.status = format!("Applied '{name}' to {}.", scope.label()),
        Err(e) => app.status = format!("error: {e}"),
    }
    let _ = app.refresh();
}

fn set_default_selected(app: &mut App) {
    let Some(p) = app.selected_profile() else {
        return;
    };
    let name = p.name.clone();
    let res = actions::set_default(p, &app.reg.profiles).and_then(|_| signers::sync(&app.reg.profiles).map(|_| ()));
    app.status = match res {
        Ok(()) => format!("'{name}' is now the global default identity."),
        Err(e) => format!("error: {e}"),
    };
    let _ = app.refresh();
}

fn toggle_auto(app: &mut App) -> Result<()> {
    let Some(p) = app.selected_profile() else {
        return Ok(());
    };
    let name = p.name.clone();
    let enabled = actions::auto_enabled(p).unwrap_or(false);
    let res = if enabled {
        actions::disable_auto(p).map(|_| format!("Auto-switch disabled for '{name}'."))
    } else {
        actions::enable_auto(p).map(|_| format!("Auto-switch enabled for '{name}'."))
    };
    app.status = match res {
        Ok(msg) => msg,
        Err(e) => format!("error: {e}"),
    };
    app.refresh()
}

fn save_edit(app: &mut App) -> Result<()> {
    let v = &app.edit.values;
    let req = |i: usize| v[i].trim().to_string();
    let opt = |i: usize| {
        let s = v[i].trim();
        (!s.is_empty()).then(|| s.to_string())
    };

    if req(F_NAME).is_empty() || req(1).is_empty() || req(2).is_empty() {
        app.status = "name, user.name and email are required.".to_string();
        return Ok(());
    }
    let signing = match (opt(6), opt(7)) {
        (Some(format), Some(key)) => {
            if format != "ssh" && format != "openpgp" {
                app.status = "sign-format must be 'ssh' or 'openpgp'.".to_string();
                return Ok(());
            }
            Some(Signing {
                format,
                key,
                auto_sign: v[F_AUTOSIGN] == "true",
            })
        }
        (None, None) => None,
        _ => {
            app.status = "sign-format and sign-key must be set together.".to_string();
            return Ok(());
        }
    };

    let ssh = opt(F_SSH_KEY).map(|key| Ssh {
        key,
        hostname: opt(F_SSH_HOST),
        host_alias: opt(F_SSH_ALIAS),
    });

    let profile = Profile {
        name: req(F_NAME),
        user_name: req(1),
        user_email: req(2),
        host: opt(3),
        username: opt(4),
        remote_match: opt(5),
        signing,
        ssh,
    };

    if app.edit.adding {
        if let Err(e) = app.reg.add(profile) {
            app.status = format!("error: {e}");
            return Ok(());
        }
    } else {
        // Replace the existing profile in place (name is read-only here).
        let orig = app.edit.original_name.clone();
        if let Some(slot) = app.reg.profiles.iter_mut().find(|p| p.name == orig) {
            *slot = profile;
        }
    }
    app.reg.save()?;
    let saved = app.edit.values[F_NAME].trim().to_string();
    // Keep ~/.ssh/config and allowed-signers in sync with the saved profile.
    if let Some(p) = app.reg.get(&saved) {
        let _ = sshconfig::apply_profile(p);
    }
    let _ = signers::sync(&app.reg.profiles);
    app.refresh()?;
    // Keep the cursor on the profile we just saved.
    if let Some(idx) = app.rows.iter().position(|r| r.name == saved) {
        app.sel = idx;
    }
    app.screen = Screen::List;
    app.status = format!("Saved '{saved}'.");
    Ok(())
}

fn delete_profile(app: &mut App, name: &str) -> Result<()> {
    match app.reg.remove(name) {
        Ok(removed) => {
            // Tear down auto-switch wiring + include file + ssh alias block.
            let _ = actions::disable_auto(&removed);
            if let Ok(path) = Registry::include_path(name) {
                let _ = std::fs::remove_file(path);
            }
            let _ = sshconfig::remove(name);
            let _ = signers::sync(&app.reg.profiles);
            app.reg.save()?;
            app.refresh()?;
            app.status = format!("Deleted '{name}'.");
        }
        Err(e) => app.status = format!("error: {e}"),
    }
    Ok(())
}

fn fix_all(app: &mut App) -> Result<()> {
    let mut fixed = 0;
    for f in &app.findings {
        if f.is_fixable() {
            f.apply()?;
            fixed += 1;
        }
    }
    app.findings = doctor::audit()?;
    app.refresh()?;
    app.status = format!("Applied {fixed} fix(es). Remember to rotate any exposed token.");
    Ok(())
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    match app.screen {
        Screen::List => draw_list(f, app, chunks[0]),
        Screen::Edit => draw_edit(f, app, chunks[0]),
        Screen::Doctor => draw_doctor(f, app, chunks[0]),
    }

    let status = Paragraph::new(Line::from(app.status.as_str()))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(status, chunks[1]);

    if let Some(c) = &app.confirm {
        draw_confirm(f, &c.prompt);
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|r| {
            let mark = if r.active { "● " } else { "  " };
            let auto = if r.auto { " ⇄" } else { "" };
            let line = Line::from(vec![
                Span::styled(mark, Style::default().fg(Color::Green)),
                Span::raw(format!("{:<14}", r.name)),
                Span::styled(auto, Style::default().fg(Color::Cyan)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" profiles "))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.sel));
    }
    f.render_stateful_widget(list, cols[0], &mut state);

    let detail = match app.selected_profile() {
        Some(p) => profile_detail(p, app.rows.get(app.sel)),
        None => Text::from("No profiles yet.\n\nPress 'n' to add one."),
    };
    let para = Paragraph::new(detail)
        .block(Block::default().borders(Borders::ALL).title(" detail "))
        .wrap(Wrap { trim: false });
    f.render_widget(para, cols[1]);
}

fn profile_detail(p: &Profile, row: Option<&Row>) -> Text<'static> {
    let mut lines = vec![
        kv("name", &p.name),
        kv("user.name", &p.user_name),
        kv("user.email", &p.user_email),
        kv("host", p.host.as_deref().unwrap_or("—")),
        kv("username", p.username.as_deref().unwrap_or("—")),
        kv("remote-match", p.remote_match.as_deref().unwrap_or("—")),
    ];
    match &p.signing {
        Some(s) => lines.push(kv(
            "signing",
            &format!("{} key={} auto-sign={}", s.format, s.key, s.auto_sign),
        )),
        None => lines.push(kv("signing", "—")),
    }
    match &p.ssh {
        Some(ssh) => {
            let desc = match &ssh.host_alias {
                Some(alias) => format!("alias {alias} → {}", actions::ssh_hostname(ssh)),
                None => "core.sshCommand".to_string(),
            };
            lines.push(kv("ssh", &format!("{desc} key={}", ssh.key)));
        }
        None => lines.push(kv("ssh", "—")),
    }
    if let Some(r) = row {
        lines.push(Line::from(""));
        let active = if r.active { "yes" } else { "no" };
        let auto = if r.auto { "enabled" } else { "disabled" };
        lines.push(kv("active (global)", active));
        lines.push(kv("auto-switch", auto));
    }
    Text::from(lines)
}

fn kv(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:<16}"), Style::default().fg(Color::Yellow)),
        Span::raw(value.to_string()),
    ])
}

fn draw_edit(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.edit.adding {
        " new profile "
    } else {
        " edit profile "
    };
    let mut lines = Vec::new();
    for (i, (label, hint)) in FIELDS.iter().enumerate() {
        let focused = i == app.edit.focus;
        let readonly = !app.edit.adding && i == F_NAME;
        let value = &app.edit.values[i];
        let shown = if i == F_AUTOSIGN {
            value.clone()
        } else if focused {
            format!("{value}█")
        } else {
            value.clone()
        };
        let label_style = if focused {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else if readonly {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let mut spans = vec![
            Span::styled(format!(" {label:<14}"), label_style),
            Span::raw(" "),
            Span::raw(shown),
        ];
        if focused {
            spans.push(Span::styled(
                format!("   ({hint})"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }
    let para = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
}

fn draw_doctor(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    if app.findings.is_empty() {
        lines.push(Line::from(Span::styled(
            "✓ No issues found.",
            Style::default().fg(Color::Green),
        )));
    }
    for (i, finding) in app.findings.iter().enumerate() {
        let color = match finding.severity {
            doctor::Severity::Critical => Color::Red,
            doctor::Severity::Warn => Color::Yellow,
            doctor::Severity::Info => Color::Cyan,
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("[{}] ", finding.severity.tag()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{}. {}", i + 1, finding.title)),
        ]));
        for dl in finding.detail.lines() {
            lines.push(Line::from(Span::styled(
                format!("      {dl}"),
                Style::default().fg(Color::Gray),
            )));
        }
        if let Some(prompt) = finding.fix_prompt() {
            lines.push(Line::from(Span::styled(
                format!("      fix: {prompt}"),
                Style::default().fg(Color::Green),
            )));
        }
        lines.push(Line::from(""));
    }
    let para = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" doctor "))
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn draw_confirm(f: &mut Frame, prompt: &str) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(prompt)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" confirm ")
                .style(Style::default().fg(Color::White).bg(Color::Black)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn centered_rect(pct_x: u16, pct_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}
