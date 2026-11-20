"""Offline contract tests; never launch a browser or touch the desktop."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("browser_worker", Path(__file__).with_name("browser_worker.py"))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class BrowserContract(unittest.TestCase):
    def test_origin_is_exact_and_credentials_and_other_schemes_are_rejected(self):
        self.assertEqual(worker.origin("https://example.org/path?q=1"), "https://example.org:443")
        for value in ("https://user:secret@example.org/", "file:///etc/passwd", "javascript:alert(1)"):
            with self.assertRaises(ValueError):
                worker.origin(value)
        self.assertNotEqual(worker.origin("https://example.org"), worker.origin("https://example.org.evil"))

    def test_freshness_confidence_and_single_use_are_checked_before_action(self):
        config = dict(max_steps=2, max_elapsed_ms=10000, min_confidence=0.8, decision_source="caller")
        session = dict(started_ms=1000, steps=0, halted=False)
        observation = dict(captured_ms=1000, nonce="a", screen={"fingerprint":"f"},
                           questions={"action":{"criteria":{"click_target_0":"Click"}}})
        decision = dict(choice="click_target_0", confidence=1)
        self.assertIsNone(worker.refusal(observation, decision, config, session, observation, "f", 2000))
        for changed, expected in ((dict(confidence=True), "insufficient_confidence"),
                                  (dict(choice="made_up"), "unknown_action")):
            self.assertEqual(worker.refusal(observation, {**decision, **changed}, config, session,
                                            observation, "f", 2000), expected)
        self.assertEqual(worker.refusal(observation, decision, config, session, None, "f", 2000),
                         "observation_consumed")
        self.assertEqual(worker.refusal(observation, decision, config, session, observation, "changed", 2000),
                         "page_changed")
        self.assertEqual(worker.refusal(observation, decision, config, session, observation, "f", 32000),
                         "stale_observation")

    def test_private_session_handle_cannot_escape_directory(self):
        for handle in ("../outside", "a"*63, "g"*64, "/tmp/other"):
            with self.assertRaises(ValueError):
                worker.session_path(handle)

    def test_server_redirect_is_refused_before_fulfillment(self):
        class Response:
            status = 302

            def __init__(self, location):
                self.headers = {"location": location}

        class Route:
            def __init__(self, location):
                self.request = type("Request", (), {"url": "http://127.0.0.1:8001/first"})()
                self.location = location
                self.fetched = None
                self.fulfilled = False
                self.aborted = False

            def fetch(self, **options):
                self.fetched = options
                return Response(self.location)

            def fulfill(self, **_):
                self.fulfilled = True

            def abort(self, _):
                self.aborted = True

        for location in ("/same-origin", "http://127.0.0.1:8002/outside"):
            browser = worker.Browser.__new__(worker.Browser)
            browser.allowed = {"http://127.0.0.1:8001"}
            browser.blocked = False
            route = Route(location)
            browser.route(route)
            self.assertEqual(route.fetched["max_redirects"], 0)
            self.assertTrue(route.aborted)
            self.assertFalse(route.fulfilled)
            self.assertTrue(browser.blocked)


if __name__ == "__main__":
    unittest.main()
