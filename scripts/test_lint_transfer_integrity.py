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

    def test_unanchored_ownerless_rule_fails_closed(self):
        # On GitHub a trailing `payment_input.rs` rule without owner would strip the owner
        # of girocrab-core/src/payment_input.rs; the earlier directory rule must not pass it.
        self.owners(OWNERS + "payment_input.rs\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("unsupported CODEOWNERS pattern", r.stderr)
        self.assertNotIn("lint-transfer-integrity: ok", r.stdout)

    def test_directory_only_rule_does_not_cover_file(self):
        # On GitHub `/…/payment_input.rs/` matches only a directory of that name: the earlier
        # ownerless rule for the file itself still wins.
        self.owners(OWNERS + "/girocrab-core/src/payment_input.rs\n/girocrab-core/src/payment_input.rs/ @o\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("payment_input.rs is not covered", r.stderr)
        self.assertNotIn("lint-transfer-integrity: ok", r.stdout)

    def test_directory_rule_still_covers_files_below(self):
        # A normal directory rule after an ownerless one owns every file below it again.
        self.owners(OWNERS + "/girocrab-core/src/\n/girocrab-core/src/ @o\n")
        r = self.lint()
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_double_star_pattern_fails_closed(self):
        self.owners(OWNERS + "/girocrab-core/**/payment_input.rs\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("unsupported CODEOWNERS pattern", r.stderr)

    def test_character_class_pattern_fails_closed(self):
        # The literal ownerless rule is followed by a class rule that bash would match too.
        self.owners(OWNERS + "/girocrab-core/src/payment_input.rs\n/girocrab-core/src/[p]ayment_input.rs @o\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1)
        self.assertIn("unsupported CODEOWNERS pattern", r.stderr)
        self.assertNotIn("lint-transfer-integrity: ok", r.stdout)

    def test_question_mark_and_backslash_patterns_fail_closed(self):
        for pattern in ("/girocrab-core/src/payment_inpu?.rs", "/girocrab-core/src/payment\\_input.rs"):
            with self.subTest(pattern=pattern):
                self.owners(OWNERS + pattern + " @o\n")
                r = self.lint()
                self.assertEqual(r.returncode, 1, r.stdout)
                self.assertIn("unsupported CODEOWNERS pattern", r.stderr)

    def test_malformed_owner_token_is_ownerless(self):
        for tail in ("foo", "@o foo", "o@", "@", "@o/"):
            with self.subTest(tail=tail):
                self.owners(OWNERS + "/policy/ " + tail + "\n")
                r = self.lint()
                self.assertEqual(r.returncode, 1, r.stdout)
                self.assertIn("policy/x is not covered", r.stderr)

    def test_valid_owner_forms_pass(self):
        for tail in ("@o", "@org/team", "sec@example.com", "@o @org/team sec@example.com # note"):
            with self.subTest(tail=tail):
                self.owners(OWNERS + "/policy/ " + tail + "\n")
                r = self.lint()
                self.assertEqual(r.returncode, 0, r.stderr)

    def test_wildcard_does_not_cross_directories(self):
        # The ownerless connectors/ rule is the last one that matches the nested file; the
        # single-level `*.rs` rule after it must not give connectors/fints/sepa.rs an owner.
        self.owners(OWNERS + "/girocrab-core/src/connectors/\n/girocrab-core/src/*.rs @dr-prinz\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("connectors/fints/sepa.rs is not covered", r.stderr)

    def test_ownerless_exception_for_real_workflow_fails(self):
        wf = self.repo / ".github/workflows/pr-checklist.yml"
        wf.parent.mkdir(parents=True, exist_ok=True)
        wf.write_text("name: x\n")
        self.commit("add workflow")
        self.owners(OWNERS + "/.github/workflows/pr-checklist.yml\n")
        r = self.lint()
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn(".github/workflows/pr-checklist.yml is not covered", r.stderr)

    def test_each_payment_field_needs_checklist(self):
        lines = {
            "a.rs": "let creditor_iban = 1;\n",
            "b.swift": "let creditorIban = 1\n",
            "c.rs": "// pain.001 export\n",
        }
        for name, line in lines.items():
            with self.subTest(file=name):
                # Back to the base: earlier cases must not leak into this case's diff.
                self.git("reset", "-q", "--hard", "main")
                (self.repo / "girocrab-core/src" / name).write_text(line)
                self.commit(f"touch {name}")
                r = self.lint(GITHUB_BASE_REF="main", PR_BODY="")
                self.assertEqual(r.returncode, 1, r.stdout)
                self.assertIn("checklist item", r.stderr)
                r = self.lint(GITHUB_BASE_REF="main", PR_BODY=TICKED)
                self.assertEqual(r.returncode, 0, r.stderr)

    def test_payment_field_outside_payment_files_needs_checklist(self):
        (self.repo / "girocrab-core/src/other.rs").write_text("let amountCents = 1;\n")
        self.commit("touch payment field elsewhere")
        r = self.lint(GITHUB_BASE_REF="main", PR_BODY="")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("checklist item", r.stderr)
        r = self.lint(GITHUB_BASE_REF="main", PR_BODY=TICKED)
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_payment_field_in_docs_needs_no_checklist(self):
        (self.repo / "README.md").write_text("creditor_iban and amount_cents\n")
        self.commit("docs mention fields")
        r = self.lint(GITHUB_BASE_REF="main", PR_BODY="")
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
