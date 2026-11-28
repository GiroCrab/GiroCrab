"""Tests for the CodeRabbit flow helper: python3 -m unittest scripts/test_coderabbit_flow.py"""
import contextlib
import io
import json
import os
import sys
import unittest
import urllib.error
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import coderabbit_flow  # noqa: E402


def thread(thread_id, outdated):
    return {"id": thread_id, "isResolved": False, "isOutdated": outdated, "path": "docs/a.md", "line": 1,
            "comments": {"nodes": [{"body": f"Finding {thread_id}", "author": {"login": "coderabbitai"}}]}}


class Api:
    """Answers the GitHub calls of one run."""

    def __init__(self, threads, resolve_error=None, comment_status=None):
        self.threads, self.resolve_error, self.comment_status = threads, resolve_error, comment_status
        self.comments = []

    def urlopen(self, req, timeout=None):
        payload = json.loads(req.data) if req.data else None
        if req.full_url.endswith("/graphql"):
            if "resolveReviewThread" not in payload["query"]:
                answer = {"data": {"repository": {"pullRequest": {
                    "state": "OPEN", "author": {"login": "someone"}, "reviewThreads": {"nodes": self.threads}}}}}
            elif self.resolve_error:
                answer = {"data": {"resolveReviewThread": None}, "errors": [{"type": self.resolve_error}]}
            else:
                answer = {"data": {"resolveReviewThread": {"thread": {"isResolved": True}}}}
        elif req.get_method() == "GET":
            answer = []
        elif self.comment_status:
            raise urllib.error.HTTPError(req.full_url, self.comment_status, "error", {}, None)
        else:
            self.comments.append(payload["body"])
            answer = {}
        return contextlib.closing(io.BytesIO(json.dumps(answer).encode()))


class FlowTests(unittest.TestCase):
    def run_main(self, api, **env):
        env = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "1", "EVENT_NAME": "pull_request_target", **env}
        out = io.StringIO()
        with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(sys, "argv", ["coderabbit_flow.py"]), \
                mock.patch("urllib.request.urlopen", api.urlopen), contextlib.redirect_stdout(out):
            return coderabbit_flow.main(), out.getvalue()

    def test_resolves_outdated_threads(self):
        api = Api([thread("a", True)])
        code, out = self.run_main(api)
        self.assertEqual(code, 0)
        self.assertIn("0 open, 1 auto-resolved", out)
        self.assertIn("All CodeRabbit review threads are resolved", api.comments[0])

    def test_forbidden_resolve_warns_and_lists_the_thread_as_open(self):
        api = Api([thread("a", True), thread("b", False)], resolve_error="FORBIDDEN")
        code, out = self.run_main(api)
        self.assertEqual(code, 0)
        self.assertIn("::warning::", out)
        self.assertIn("2 open, 0 auto-resolved", out)
        self.assertIn("Finding a", api.comments[0])
        self.assertIn("Finding b", api.comments[0])

    def test_resolve_command_does_not_claim_success_when_forbidden(self):
        api = Api([thread("a", False)], resolve_error="FORBIDDEN")
        code, _ = self.run_main(api, EVENT_NAME="issue_comment", COMMENT_BODY="/cr-resolve", AUTHOR_ASSOC="OWNER")
        self.assertEqual(code, 0)
        self.assertEqual(len(api.comments), 1)
        self.assertNotIn("All CodeRabbit review threads are resolved", api.comments[0])

    def test_confirmed_resolution_refreshes_the_status_when_forbidden(self):
        api = Api([thread("a", True), thread("b", False)], resolve_error="FORBIDDEN")
        code, out = self.run_main(api, EVENT_NAME="issue_comment", COMMENT_AUTHOR="coderabbitai[bot]",
                                  COMMENT_BODY="Comments resolved")
        self.assertEqual(code, 0)
        self.assertIn("0/2 thread(s) resolved", out)
        self.assertEqual(len(api.comments), 1)
        self.assertIn("Finding a", api.comments[0])
        self.assertIn("Finding b", api.comments[0])

    def test_read_only_token_warns_about_the_status_comment(self):
        code, out = self.run_main(Api([thread("a", False)], comment_status=403))
        self.assertEqual(code, 0)
        self.assertIn("::warning::Could not write the status comment", out)

    def test_other_errors_still_fail(self):
        with self.assertRaises(RuntimeError):
            self.run_main(Api([thread("a", True)], resolve_error="NOT_FOUND"))
        with self.assertRaises(urllib.error.HTTPError):
            self.run_main(Api([thread("a", False)], comment_status=500))

    def test_summarizes_and_extracts_prompt_from_picture_badge_format(self):
        body = (
            "<!-- coderabbit-ai-generated:v1 -->\n\n"
            '**<picture><img src="https://storage.googleapis.com/coderabbit_public_assets/review-comment-icons-v1/amber-16.svg" '
            'width="16" height="16" align="absmiddle" alt=""></picture> Minor** · **Data Integrity** · **Quick win**\n\n'
            "**Record the failure properly.**\n\n"
            "Explanation paragraph.\n\n"
            "<details>\n"
            '<summary><picture><img src="https://example.com/bot.svg" alt=""></picture> Prompt to fix issue</summary>\n'
            "<dl>\n<dd>\n\n"
            "```\nFix the bug in foo.rs.\n```\n\n"
            "</dd>\n</dl>\n</details>\n"
        )
        self.assertEqual(coderabbit_flow.summarize_comment(body), "**Record the failure properly.**")
        self.assertEqual(coderabbit_flow.extract_ai_prompt(body), "Fix the bug in foo.rs.")

    def test_extracts_legacy_prompt_and_does_not_cross_details_boundary(self):
        legacy = (
            "**Fix null check.**\n\n"
            "<details>\n<summary>🤖 Prompt for AI Agents</summary>\n\n"
            "```text\nCheck for None before unwrap.\n```\n</details>\n"
        )
        self.assertEqual(coderabbit_flow.summarize_comment(legacy), "**Fix null check.**")
        self.assertEqual(coderabbit_flow.extract_ai_prompt(legacy), "Check for None before unwrap.")

        no_code_in_details = (
            "<details><summary>Prompt to fix issue</summary>No fence here.</details>\n"
            "```\nUnrelated later snippet\n```\n"
        )
        self.assertEqual(coderabbit_flow.extract_ai_prompt(no_code_in_details), "")


if __name__ == "__main__":
    unittest.main()
