"""Unit tests for tools/check-security-checklist.py (run: python3 -m unittest tools/test_check_security_checklist.py)."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "chk", pathlib.Path(__file__).with_name("check-security-checklist.py"))
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)

CRITICAL = [
    "girocrab-core/src/vault.rs", "girocrab-core/src/vault/bookings.rs", "girocrab-core/src/secret_store.rs",
    "girocrab-core/src/lib.rs", "girocrab-core/src/sepa.rs", "girocrab-core/src/payment_input.rs",
    "girocrab-core/src/connectors/fints/mod.rs", "girocrab-core/src/local_api/server.rs",
    "GiroCrabApp/Sources/GiroCrabUI/VaultKeyManager.swift", "GiroCrabApp/Sources/GiroCrabUI/BankingPolicy.swift",
    ".github/workflows/ci.yml", "scripts/lint-fonts.sh",
]


class SecurityChecklist(unittest.TestCase):
    def test_every_critical_path_needs_the_check(self):
        for p in CRITICAL:
            ok, msg = chk.verdict(["README.md", p], "no checklist")
            self.assertFalse(ok, p)
            self.assertIn(p, msg)

    def test_other_paths_pass_without_body(self):
        ok, _ = chk.verdict(["girocrab-core/src/tags.rs", "GiroCrabApp/Sources/GiroCrabUI/RootView.swift", "docs/x.md"], "")
        self.assertTrue(ok)

    def test_master_key_generator_is_gated(self):
        for body in ("", None):
            ok, msg = chk.verdict(["girocrab-core/src/lib.rs"], body)
            self.assertFalse(ok, body)
            self.assertIn("girocrab-core/src/lib.rs", msg)

    def test_ticked_and_explained_check_passes(self):
        for body in ("- [x] **Security-Check:** Nothing relevant", "* [X] Security-Check: ok",
                     "  - [x] Security-Check keine neuen Zugriffe"):
            self.assertTrue(chk.verdict(CRITICAL, body)[0], body)

    def test_ticked_without_explanation_fails(self):
        for body in ("  - [x] Security-Check", "- [x] **Security-Check:**", "- [x] **Security-Check:**   ",
                     "- [x] **Security-Check:** <!-- Pflicht bei Änderungen … -->",
                     "- [x] **Security-Check:**\n- **Perf-Check:** ok"):
            self.assertFalse(chk.verdict(CRITICAL, body)[0], body)

    def test_unticked_or_plain_or_commented_fails(self):
        for body in ("- [ ] **Security-Check:**", "- **Security-Check:** all fine", "- [x] Perf-Check: ok",
                     "<!-- - [x] **Security-Check:** -->", "", None):
            self.assertFalse(chk.verdict(CRITICAL, body)[0], body)

    def test_ticked_example_in_fenced_block_does_not_count(self):
        for body in ("```\n- [x] **Security-Check:** example\n```\n- [ ] **Security-Check:**",
                     "~~~md\n- [x] **Security-Check:** example\n~~~\n- [ ] **Security-Check:**",
                     "- [ ] **Security-Check:**\n```\n- [x] **Security-Check:** unclosed example"):
            self.assertFalse(chk.verdict(CRITICAL, body)[0], body)

    def test_real_check_after_fenced_example_passes(self):
        body = "```\n- [ ] **Security-Check:** example\n```\n- [x] **Security-Check:** Nothing relevant: nur CI"
        self.assertTrue(chk.verdict(CRITICAL, body)[0])

    def test_message_names_the_fix(self):
        msg = chk.verdict(["scripts/x.sh"], "")[1]
        self.assertIn("- [x] **Security-Check:**", msg)


if __name__ == "__main__":
    unittest.main()
