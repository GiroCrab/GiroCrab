"""Tests for scripts/analyse-dokumente.py: python3 -m unittest scripts/test_analyse_dokumente.py"""
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("analyse", os.path.join(HERE, "analyse-dokumente.py"))
ad = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ad)
RULES = os.path.join(HERE, "..", "girocrab-core", "data", "document_rules.json")


class RulesFile(unittest.TestCase):
    def setUp(self):
        ad.load_rules(RULES)

    def test_the_rules_file_parses_and_has_every_section(self):
        with open(RULES, encoding="utf-8") as f:
            data = json.load(f)
        for key in ("dokumenttypen", "standard", "arten", "banken"):
            self.assertIn(key, data)

    def test_a_bank_is_recognised_by_its_words(self):
        self.assertEqual(ad.guess_bank("x.pdf", "comdirect bank ag", ""), "comdirect")
        self.assertTrue(ad.guess_bank("x.pdf", "irgendwer", "").startswith("unbekannt"))

    def test_bank_own_types_win_over_the_general_ones(self):
        self.assertEqual(ad.guess_type("", "wertpapierkauf geschäftsnummer", "comdirect"), "Kauf")
        self.assertEqual(ad.guess_type("", "wertpapier abrechnung kauf", "DKB"), "Kauf")
        own = {name: [t for t, _p in types] for name, _k, types in ad.BANKS}
        self.assertEqual(own["comdirect"], ["Verkauf", "Kauf", "Steuer"])
        self.assertEqual(own["ING"], [])
        self.assertEqual(ad.guess_type("wertpapierabrechnung_verkauf_x.pdf", "", ""), "Verkauf")

    def test_every_art_names_the_app_type_it_maps_to(self):
        with open(RULES, encoding="utf-8") as f:
            data = json.load(f)
        valid = {"steuer", "kontoauszug", "abrechnung", "postbox", "beleg"}
        for art in data["arten"] + [a for b in data["banken"] for a in b.get("arten", [])]:
            self.assertIn(art.get("typ"), valid, art["name"])

    def test_the_shared_examples_give_the_expected_art(self):
        # the same titles the core checks against `typ` (documents.rs, the_shared_examples_of_the_rules_file…)
        with open(RULES, encoding="utf-8") as f:
            examples = json.load(f)["pruefbeispiele"]
        self.assertGreaterEqual(len(examples), 15)
        for e in examples:
            self.assertEqual(ad.guess_type(e["titel"].lower(), "", e["bank"]), e["art"], e["titel"])

    def test_an_unknown_text_is_other(self):
        self.assertEqual(ad.guess_type("", "nichts bekanntes", ""), "Sonstiges")


class Redaction(unittest.TestCase):
    def setUp(self):
        self.r = ad.Redactor(["Max Mustermann"])

    def test_names_ibans_mails_and_long_numbers_go(self):
        line = self.r.line("Max Mustermann DE89370400440532013000 max@example.org Depotnummer 123456789")
        for secret in ("Mustermann", "DE89", "example.org", "123456789"):
            self.assertNotIn(secret, line)

    def test_amounts_dates_and_isin_stay(self):
        line = self.r.line("Kauf IE00B3RBWM25 am 23.07.2021 über 1.234,56 EUR")
        for kept in ("IE00B3RBWM25", "23.07.2021", "1.234,56"):
            self.assertIn(kept, line)

    def test_a_first_name_next_to_a_name_is_collapsed(self):
        self.assertEqual(self.r.line("Max Mustermann"), "[Name]")
        self.assertNotIn("Erika", self.r.line("Erika Max Mustermann"))

    def test_the_address_block_before_the_postcode_line_is_hidden(self):
        text = "Depotnummer 1\nHerr Beispiel\nBeispielweg 5\n12345 Berlin\nWertpapier Abrechnung Kauf\n"
        lines = ad.head_lines(text, 10, self.r)
        self.assertNotIn("Beispielweg", "\n".join(lines))
        self.assertIn("Wertpapier Abrechnung Kauf", lines)

    def test_a_report_that_still_names_someone_is_refused(self):
        self.assertTrue(self.r.leaks("Mustermann"))
        self.assertTrue(self.r.leaks("[Name] Erika"))
        self.assertFalse(self.r.leaks("[Name]  Depotnummer"))


if __name__ == "__main__":
    unittest.main()
