# typed: false
# frozen_string_literal: true

# Homebrew formula for smelt, the standalone CLI + LSP.
#
# This file lives in-repo and is copied (or symlinked) into the
# `adbrowne/homebrew-smelt` tap. See packaging/homebrew/README.md for the
# tap bootstrap and per-release update process; the sha256 values below are
# rewritten by scripts/update-homebrew-formula.sh, never by hand.
class Smelt < Formula
  desc "Modern data transformation framework"
  homepage "https://github.com/adbrowne/smelt-sql"
  version "0.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/adbrowne/smelt-sql/releases/download/v#{version}/smelt-macos-aarch64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/adbrowne/smelt-sql/releases/download/v#{version}/smelt-linux-x86_64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000"
    end
    on_arm do
      url "https://github.com/adbrowne/smelt-sql/releases/download/v#{version}/smelt-linux-aarch64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "smelt"
    bin.install "smelt-lsp"
    bin.install "smelt-datagen"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/smelt --version")
  end
end
