"""Tests for scripts/lint-transfer-integrity.sh (payment-path integrity, #1799, #946).

Each test copies the script into a throwaway repository with its own CODEOWNERS and
payment files and a bare clone as `origin`, so the PR checks run offline."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "lint-transfer-integrity.sh"
PAYMENT_FILES = [
    "GiroCrabApp/Sources/GiroCrabUI/TransferSheetView.swift",
    "girocrab-core/src/payment_input.rs",
    "girocrab-core/src/connectors/fints/sepa.rs",
    "girocrab-core/src/connectors/fints/transfer.rs",
    "girocrab-core/src/connectors/fints/debit.rs",
    "girocrab-core/src/connectors/fints/standing.rs",
]
OWNERS = "/girocrab-core/ @o\n/GiroCrabApp/ @o\n/.github/ @o\n/scripts/ @o\n/policy/ @o\n"
TICKED = "- [x] Zahlungspfad geprüft / payment path reviewed: IBAN-/Betragsprüfung unverändert\n"
GIT_ENV = {
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}


class LintTransferIntegrityTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.repo = Path(tmp.name) / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        shutil.copy(SCRIPT, self.repo / "scripts")
        for f in PAYMENT_FILES:
            p = self.repo / f
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("ok\n")
        self.owners(OWNERS)
        self.git("init", "-q", "-b", "main")
        self.commit("base")
        origin = Path(tmp.name) / "origin.git"
        subprocess.run(
            ["git", "clone", "-q", "--bare", str(self.repo), str(origin)],
            check=True, capture_output=True, env=self.env(),
        )
        self.git("remote", "add", "origin", origin.as_uri())
        self.git("checkout", "-q", "-b", "pr")

    def env(self, **extra):
        env = {k: v for k, v in os.environ.items() if k not in ("GITHUB_BASE_REF", "PR_BODY")}
        env.update(GIT_ENV, **extra)
        return env

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True, env=self.env())

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def owners(self, text):
        p = self.repo / ".github" / "CODEOWNERS"
        p.parent.mkdir(exist_ok=True)
        p.write_text(text)

    def lint(self, **extra):
        return subprocess.run(
            ["bash", "scripts/lint-transfer-integrity.sh"],
            cwd=self.repo, capture_output=True, text=True, env=self.env(**extra),
        )

    def touch_payment_path(self):
        (self.repo / "girocrab-core/src/payment_input.rs").write_text("changed\n")
        self.commit("change payment input")

    def test_owned_paths_pass_outside_a_pr(self):
        r = self.lint()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("lint-transfer-integrity: ok", r.stdout)

    def test_later_ownerless_rule_wins(self):
        self.owners(OWNERS + "/girocrab-core/src/payment_input.rs\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("payment_input.rs is not covered", r.stderr)

    def test_comment_instead_of_owner_is_ownerless(self):
        self.owners(OWNERS + "/policy/ # owner to follow\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("policy/x is not covered", r.stderr)

    def test_codeowners_itself_must_have_an_owner(self):
        self.owners(OWNERS + "/.github/CODEOWNERS\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn(".github/CODEOWNERS is not covered", r.stderr)

    def test_payment_pr_with_ticked_item_passes(self):
        self.touch_payment_path()
        r = self.lint(GITHUB_BASE_REF="main", PR_BODY="## Zahlungspfad\r\n" + TICKED)
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_payment_pr_without_ticked_item_fails(self):
        self.touch_payment_path()
        for body in ("", TICKED.replace("[x]", "[ ]"), "- [x] Update payment icon\n"):
            with self.subTest(body=body):
                r = self.lint(GITHUB_BASE_REF="main", PR_BODY=body)
                self.assertEqual(r.returncode, 1, r.stdout)
                self.assertIn("checklist item", r.stderr)

    def test_pr_outside_payment_path_needs_no_checklist(self):
        (self.repo / "README.md").write_text("docs\n")
        self.commit("docs")
        r = self.lint(GITHUB_BASE_REF="main", PR_BODY="")
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_unreachable_base_fails_in_pr_context(self):
        self.touch_payment_path()
        r = self.lint(GITHUB_BASE_REF="no-such-branch", PR_BODY=TICKED)
        self.assertEqual(r.returncode, 1)
        self.assertIn("cannot fetch base", r.stderr)


if __name__ == "__main__":
    unittest.main()
