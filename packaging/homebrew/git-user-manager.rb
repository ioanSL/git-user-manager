# Homebrew formula for git-user-manager (gum).
#
# Builds from source so it works on both macOS and Linux with no prebuilt
# per-OS binaries. Before publishing a tagged release:
#   1. Tag and push (vX.Y.Z), then run:
#        packaging/homebrew/update-sha.sh ioanSL X.Y.Z
#      to fill in the url version and sha256.
#
# Until a release is tagged, users can still install the latest with:
#   brew install --HEAD <tap>/git-user-manager
class GitUserManager < Formula
  desc "Manage multiple git identities (GitHub/GitLab users) with auto-switching"
  homepage "https://github.com/ioanSL/git-user-manager"
  url "https://github.com/ioanSL/git-user-manager/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"
  head "https://github.com/ioanSL/git-user-manager.git", branch: "main"

  depends_on "rust" => :build
  depends_on "git"

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    # Version string (regex tolerates --HEAD builds where version != tag).
    assert_match(/gum \d+\.\d+\.\d+/, shell_output("#{bin}/gum --version"))

    # Smoke test the registry against an isolated HOME so we never touch the
    # real git config.
    ENV["HOME"] = testpath
    output = shell_output("#{bin}/gum list")
    assert_match "No profiles yet", output

    system bin/"gum", "add", "work",
           "--user-name", "Ada", "--email", "ada@example.com"
    assert_match "work", shell_output("#{bin}/gum list")
  end
end
