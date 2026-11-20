"""Account-free contract tests; import does not load desktop frameworks."""
import copy
import io
import importlib.util
import pathlib
import json
import os
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("apr_computer_worker", pathlib.Path(__file__).with_name("computer_worker.py"))
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


def config():
    return {"allowed_apps": ["org.apr.Fixture"], "max_steps": 3,
            "max_elapsed_ms": 60000, "min_confidence": 0.8, "model": "jev-latest",
            "allowed_text": {"query": "hello"}, "decision_source": "caller"}


def screen():
    return {"app": "org.apr.Fixture", "pid": 42,
            "window": {"id": 7, "bounds": [20, 30, 800, 600]},
            "targets": [{"label": "Continue", "bounds": [40, 50, 100, 30]}],
            "focus": {"role": "AXTextField", "subrole": "", "value": "must-never-leak",
                      "label": "Query", "placeholder": "Search", "identifier": "query",
                      "bounds": [40, 50, 100, 30]}}


class Backend:
    def __init__(self, value=None, changes=True):
        self.value = value or screen()
        self.actions = []
        self.observations = 0
        self.changes = changes
        self.fail = False

    def observe(self, allowed):
        self.observations += 1
        return copy.deepcopy(self.value)

    def perform(self, action, current, settings):
        self.actions.append(action)
        if self.fail:
            raise RuntimeError("effect happened, reply lost")
        if self.changes:
            self.value["targets"][0]["label"] = "Observed change " + str(len(self.actions))


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.config = config()
        self.backend = Backend()
        self.now = 100000

    def observe(self, session=None):
        return worker.observe({"goal": "Review synthetic record", "session": session},
                              self.config, self.backend, lambda: self.now)

    def act(self, observation, choice="click_target_0", confidence=0.95):
        return worker.act({"observation": observation, "decision": {"choice": choice, "confidence": confidence}},
                          self.config, self.backend, lambda: self.now)

    def test_window_state_is_bounded_and_excludes_accessibility_values(self):
        self.backend.value["targets"] *= 100
        observation = self.observe()
        self.assertEqual(len(observation["screen"]["targets"]), 22)
        self.assertEqual(observation["screen"]["omitted_targets"], 78)
        self.assertLessEqual(len(observation["questions"]["action"]["criteria"]), 32)
        self.assertNotIn("must-never-leak", str(observation))
        self.assertNotIn("value", observation["screen"]["focus"])

    def test_changes_to_omitted_targets_still_invalidate_observation(self):
        self.backend.value["targets"] = [copy.deepcopy(screen()["targets"][0]) for _ in range(30)]
        observation = self.observe()
        self.backend.value["targets"][-1]["label"] = "changed off candidate list"
        result = self.act(observation)
        self.assertEqual(result["reason"], "screen_changed")
        self.assertEqual(self.backend.actions, [])

    def test_allowed_action_observes_again_and_records_change_without_claiming_success(self):
        result = self.act(self.observe())
        self.assertEqual(self.backend.actions, ["click_target_0"])
        self.assertEqual(self.backend.observations, 3)
        self.assertFalse(result["halted"])
        self.assertTrue(result["last_action"]["screen_changed"])
        self.assertFalse(result["last_action"]["verified"])

    def test_unknown_target_or_arbitrary_operation_is_never_dispatched(self):
        observation = self.observe()
        for choice in ("click_target_999", "open_https://example.com", "type_unregistered", "shell"):
            self.assertEqual(self.act(observation, choice)["reason"], "unknown_action")
        self.assertEqual(self.backend.actions, [])

    def test_missing_low_nonfinite_or_invalid_confidence_escalates(self):
        observation = self.observe()
        for confidence in (None, 0.7, float("nan"), float("inf"), -0.1, 1.1, True, "0.95"):
            self.assertEqual(self.act(observation, confidence=confidence)["reason"], "insufficient_confidence")
        self.assertEqual(self.backend.actions, [])

    def test_model_done_is_only_a_verification_request(self):
        result = self.act(self.observe(), "done")
        self.assertEqual(result["reason"], "verification_required")
        self.assertTrue(result["halted"])
        self.assertEqual(self.backend.actions, [])

    def test_explicit_escalation_never_calls_a_writer(self):
        self.assertEqual(self.act(self.observe(), "escalate")["reason"], "caller_requested")
        self.assertEqual(self.backend.actions, [])

    def test_stale_observation_and_backwards_clock_are_rejected(self):
        observation = self.observe()
        for timestamp in (130001, 99999):
            self.now = timestamp
            self.assertEqual(self.act(observation)["reason"], "stale_observation")
        self.assertEqual(self.backend.actions, [])

    def test_session_deadline_counts_pause_time(self):
        self.config["max_elapsed_ms"] = 1000
        observation = self.observe()
        self.now += 1001
        self.assertEqual(self.act(observation)["reason"], "elapsed_limit")
        self.assertEqual(self.backend.actions, [])

    def test_app_or_window_changed_refuses_action(self):
        observation = self.observe()
        self.backend.value["app"] = "org.other.App"
        self.assertEqual(self.act(observation)["reason"], "application_not_allowed")
        self.backend.value = screen()
        self.backend.value["window"]["id"] += 1
        self.assertEqual(self.act(observation)["reason"], "screen_changed")
        self.assertEqual(self.backend.actions, [])

    def test_disallowed_app_cannot_supply_an_observation(self):
        self.backend.value["app"] = "org.other.App"
        result = self.observe()
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"]["reason"], "application_not_allowed")

    def test_secure_fields_and_noneditable_fields_never_offer_typing(self):
        for field in ({"role": "AXTextField", "subrole": "AXSecureTextField"},
                      {"role": "AXSecureTextField"}, {"role": "AXButton"}):
            self.backend.value["focus"] = field
            observation = self.observe()
            self.assertNotIn("type_query", observation["questions"]["action"]["criteria"])
            self.assertEqual(self.act(observation, "type_query")["reason"], "unknown_action")
        self.assertEqual(self.backend.actions, [])

    def test_secure_focus_blocks_keyboard_and_click_dispatch(self):
        self.backend.value["focus"] = {"role": "AXTextField", "subrole": "AXSecureTextField"}
        self.assertEqual(self.act(self.observe(), "press_enter")["reason"], "secure_field")
        self.assertEqual(self.backend.actions, [])

    def test_post_dispatch_failure_escapes_for_apr_uncertainty_handling(self):
        self.backend.fail = True
        with self.assertRaises(worker.WorkerFailure) as failure:
            self.act(self.observe())
        self.assertEqual(failure.exception.exit_code, worker.EXIT_EVENT_DISPATCH)
        self.assertEqual(self.backend.actions, ["click_target_0"])

    def test_step_limit_and_no_change_limit_halt_deterministically(self):
        self.config["max_steps"] = 1
        self.assertEqual(self.act(self.observe())["reason"], "step_limit")
        self.config["max_steps"] = 3
        self.backend.changes = False
        session = self.act(self.observe())
        self.assertFalse(session["halted"])
        self.assertEqual(self.act(self.observe(session))["reason"], "repeated_no_change")

    def test_halted_session_never_calls_desktop_backend(self):
        session = {"halted": True, "reason": "caller_requested"}
        result = self.observe(session)
        self.assertFalse(result["ready"])
        self.assertEqual(self.backend.observations, 0)

    def test_jev_normalized_observation_uses_explicit_confidence(self):
        self.config["decision_source"] = "jev"
        observation = self.observe()
        request = {"observation": observation, "decision": {"answers": {"action": {
            "choice": "click_target_0", "probabilities": {"click_target_0": 1}}}}}
        self.assertEqual(worker.act(request, self.config, self.backend, lambda: self.now)["reason"], "insufficient_confidence")
        request["decision"]["answers"]["action"]["confidence"] = 0.9
        self.assertEqual(worker.act(request, self.config, self.backend, lambda: self.now)["steps"], 1)

    def test_text_catalog_and_session_policy_reject_invalid_inputs(self):
        for updates in ({"allowed_apps": []}, {"max_steps": 13}, {"max_elapsed_ms": 0},
                        {"min_confidence": float("nan")}, {"allowed_text": {"query": "foo\nbar"}},
                        {"allowed_text": {"query": "x" * 257}}):
            with self.assertRaises(ValueError):
                worker.policy({**config(), **updates})

    def test_focus_geometry_and_identity_change_refuse_typing(self):
        for field, changed in (("bounds", [40, 100, 100, 30]), ("identifier", "other-field")):
            self.backend.value = screen()
            observation = self.observe()
            self.backend.value["focus"][field] = changed
            self.assertEqual(self.act(observation, "type_query")["reason"], "screen_changed")
        self.assertEqual(self.backend.actions, [])

    def test_declared_exact_visible_condition_completes_without_a_model(self):
        self.config["success_text"] = "Continue"
        result = self.observe()
        self.assertFalse(result["ready"])
        self.assertTrue(result["session"]["complete_verified"])
        self.assertEqual(result["session"]["completion"]["evidence"]["text"], "Continue")
        self.assertEqual(self.backend.actions, [])

    def test_predicate_is_exact_and_does_not_match_a_truncated_label(self):
        for label, predicate in (("Completed extra", "Completed"), ("x" * 257, "x" * 256)):
            self.backend.value["targets"][0]["label"] = label
            self.config["success_text"] = predicate
            self.assertTrue(self.observe()["ready"])

    def test_completion_condition_after_last_action_takes_precedence_over_step_limit(self):
        self.config.update(max_steps=1, success_text="Observed change 1")
        result = self.act(self.observe())
        self.assertTrue(result["complete_verified"])
        self.assertEqual(result["reason"], "visible_condition_met")

    def test_unicode_observations_fit_the_actual_native_byte_budget(self):
        self.backend.value["targets"] = [{"label": "界" * 256, "bounds": [40, 50, 100, 30]} for _ in range(100)]
        with tempfile.TemporaryDirectory() as lock_directory:
            result = worker.handle({"protocol_version": "0.1", "input": {"operation": "observe", "goal": "界" * 4000},
                                    "config": self.config}, self.backend, lambda: self.now,
                                   lock_path=pathlib.Path(lock_directory) / "desktop.lock")
        self.assertTrue(result["output"]["ready"])
        self.assertLess(len(json.dumps(result, ensure_ascii=False, separators=(",", ":")).encode()), 50000)
        visible = result["output"]["screen"]
        self.assertEqual(len(visible["targets"]) + visible["omitted_targets"], 100)
        self.assertEqual(len(result["output"]["questions"]["action"]["criteria"]), len(visible["targets"]) + 7)


class Quartz:
    kCGEventMouseMoved, kCGEventLeftMouseDown, kCGEventLeftMouseUp = 1, 2, 3
    kCGMouseButtonLeft = 0

    def CGEventCreateMouseEvent(self, _, kind, point, button):
        return {"kind": kind, "point": point}

    def CGEventCreateKeyboardEvent(self, _, code, down):
        return {"code": code, "down": down}

    def CGEventKeyboardSetUnicodeString(self, event, length, char):
        event["char"] = char

    def CGEventCreate(self, _):
        return None

    def CGEventGetLocation(self, _):
        return types.SimpleNamespace(x=20, y=20)


class NativeEventTests(unittest.TestCase):
    def setUp(self):
        self.config = config()
        self.config["_deadline_ms"] = worker.now_ms() + 100000
        self.screen = worker.normalize(screen(), self.config)
        self.backend = worker.MacOSBackend.__new__(worker.MacOSBackend)
        self.backend.q = Quartz()
        self.backend._front = lambda _: (self.screen["app"], self.screen["pid"])
        self.backend._window = lambda _: self.screen["window"]
        self.focus = copy.deepcopy(self.screen["focus"])
        self.backend._focus = lambda _: self.focus
        self.backend._point_clear = lambda *_: None
        self.events = []
        self.backend._post = self.events.append

    def test_mouse_up_is_posted_even_when_down_raises_after_dispatch(self):
        def post(event):
            self.events.append(event)
            if event.get("kind") == Quartz.kCGEventLeftMouseDown:
                raise RuntimeError("after down")
        self.backend._post = post
        with self.assertRaises(RuntimeError):
            self.backend.perform("click_target_0", self.screen, self.config)
        self.assertEqual([event["kind"] for event in self.events], [1, 2, 3])

    def test_key_up_is_posted_even_when_down_raises_after_dispatch(self):
        def post(event):
            self.events.append(event)
            if event.get("down"):
                raise RuntimeError("after down")
        self.backend._post = post
        with self.assertRaises(RuntimeError):
            self.backend.perform("press_enter", self.screen, self.config)
        self.assertEqual([event["down"] for event in self.events], [True, False])

    def test_focus_change_between_characters_stops_typing_after_releasing_key(self):
        def post(event):
            self.events.append(event)
            if event.get("down"):
                self.focus["identifier"] = "other-field"
        self.backend._post = post
        with self.assertRaisesRegex(worker.Unavailable, "focus_changed"):
            self.backend.perform("type_query", self.screen, self.config)
        self.assertEqual([event["char"] for event in self.events], ["h", "h"])
        self.assertEqual([event["down"] for event in self.events], [True, False])

    def test_session_deadline_prevents_new_native_events(self):
        self.config["_deadline_ms"] = worker.now_ms() - 1
        with self.assertRaisesRegex(worker.Unavailable, "elapsed_limit"):
            self.backend.perform("press_enter", self.screen, self.config)
        self.assertEqual(self.events, [])


class NativeCoordinationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.lock_path = pathlib.Path(self.directory.name) / "desktop.lock"
        self.ready_path = pathlib.Path(self.directory.name) / "entered"
        self.release_path = pathlib.Path(self.directory.name) / "release"

    def tearDown(self):
        self.directory.cleanup()

    def _child(self, operation, observation=None):
        source = pathlib.Path(worker.__file__).resolve()
        script = r'''
import importlib.util, json, pathlib, sys, time
spec = importlib.util.spec_from_file_location("apr_computer_worker", sys.argv[1])
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
lock_path, ready_path, release_path = map(pathlib.Path, sys.argv[2:5])
operation = sys.argv[5]
config = json.loads(sys.argv[6])
observation = json.loads(sys.argv[7]) if sys.argv[7] else None
class Backend:
    def observe(self, allowed):
        if operation == "observe":
            ready_path.touch()
            deadline = time.monotonic() + 5
            while not release_path.exists() and time.monotonic() < deadline:
                time.sleep(.01)
            if not release_path.exists(): raise RuntimeError("fixture timeout secret")
        return {"app": "org.apr.Fixture", "pid": 42,
                "window": {"id": 7, "bounds": [20, 30, 800, 600]},
                "targets": [{"label": "Continue", "bounds": [40, 50, 100, 30]}],
                "focus": {"role": "AXTextField", "subrole": "", "value": "must-never-leak",
                          "label": "Query", "placeholder": "Search", "identifier": "query",
                          "bounds": [40, 50, 100, 30]}}
    def perform(self, action, screen, settings):
        ready_path.touch()
        deadline = time.monotonic() + 5
        while not release_path.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        if not release_path.exists(): raise RuntimeError("fixture timeout secret")
envelope = {"protocol_version":"0.1", "config":config,
            "input":{"operation":operation, "goal":"synthetic"}}
if operation == "act":
    envelope["input"] = {"operation":"act", "observation":observation,
                         "decision":{"choice":"click_target_0", "confidence":.95}}
worker.handle(envelope, Backend(), worker.now_ms, lock_path=lock_path)
'''
        config_json = json.dumps(config())
        observation_json = json.dumps(observation) if observation is not None else ""
        process = subprocess.Popen(
            [sys.executable, "-I", "-c", script, str(source), str(self.lock_path),
             str(self.ready_path), str(self.release_path), operation,
             config_json, observation_json],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        deadline = time.monotonic() + 5
        while not self.ready_path.exists() and process.poll() is None and time.monotonic() < deadline:
            time.sleep(.01)
        if not self.ready_path.exists():
            process.kill()
            stdout, stderr = process.communicate(timeout=2)
            self.fail(f"child did not reach fake backend; exit={process.returncode}, output lengths={(len(stdout), len(stderr))}")
        return process

    def _observe_envelope(self):
        return {"protocol_version": "0.1", "config": config(),
                "input": {"operation": "observe", "goal": "synthetic"}}

    def test_lock_excludes_other_processes_for_the_full_observe_operation(self):
        child = self._child("observe")
        backend = Backend()
        began = time.monotonic()
        result = worker.handle(self._observe_envelope(), backend, lambda: 100000,
                               lock_path=self.lock_path)["output"]
        self.assertLess(time.monotonic() - began, .5)
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"]["reason"], "desktop_busy")
        self.assertEqual(result["session"]["diagnostic"],
                         {"stage": "observe", "code": "desktop_busy"})
        self.assertEqual(backend.observations, 0)
        self.release_path.touch()
        stdout, stderr = child.communicate(timeout=5)
        self.assertEqual(child.returncode, 0, (len(stdout), len(stderr)))
        self.assertEqual(self.lock_path.stat().st_mode & 0o777, 0o600)

    def test_lock_excludes_other_processes_through_action_and_post_observation(self):
        envelope = self._observe_envelope()
        observed = worker.handle(envelope, Backend(), worker.now_ms,
                                 lock_path=self.lock_path)["output"]
        child = self._child("act", observed)
        backend = Backend()
        began = time.monotonic()
        result = worker.handle(envelope, backend, worker.now_ms,
                               lock_path=self.lock_path)["output"]
        self.assertLess(time.monotonic() - began, .5)
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"]["reason"], "desktop_busy")
        self.assertEqual(backend.observations, 0)
        self.release_path.touch()
        stdout, stderr = child.communicate(timeout=5)
        self.assertEqual(child.returncode, 0, (len(stdout), len(stderr)))

    def test_busy_action_does_not_wait_then_reuses_stale_observation(self):
        envelope = self._observe_envelope()
        observed = worker.handle(envelope, Backend(), lambda: 100000,
                                 lock_path=self.lock_path)["output"]
        child = self._child("observe")
        now = [100000]
        backend = Backend()
        action_envelope = {"protocol_version": "0.1", "config": config(),
                           "input": {"operation": "act", "observation": observed,
                                     "decision": {"choice": "click_target_0", "confidence": .95}}}
        result = worker.handle(action_envelope, backend, lambda: now[0],
                               lock_path=self.lock_path)["output"]
        self.assertEqual(result["reason"], "desktop_busy")
        self.assertEqual(backend.actions, [])
        self.release_path.touch()
        stdout, stderr = child.communicate(timeout=5)
        self.assertEqual(child.returncode, 0, (len(stdout), len(stderr)))
        now[0] = 130001
        stale = worker.handle(action_envelope, backend, lambda: now[0],
                              lock_path=self.lock_path)["output"]
        self.assertEqual(stale["reason"], "stale_observation")
        self.assertEqual(backend.actions, [])

    def test_shared_lock_directory_fails_closed_before_observation(self):
        self.lock_path.parent.chmod(0o755)
        backend = Backend()
        result = worker.handle(self._observe_envelope(), backend, lambda: 100000,
                               lock_path=self.lock_path)["output"]
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"]["reason"], "desktop_lock_unavailable")
        self.assertEqual(result["session"]["diagnostic"],
                         {"stage": "observe", "code": "desktop_lock_unavailable"})
        self.assertEqual(backend.observations, 0)

    def test_symlink_lock_file_fails_closed_without_touching_target(self):
        target = self.lock_path.parent / "target"
        target.write_text("unchanged")
        target.chmod(0o600)
        self.lock_path.symlink_to(target)
        backend = Backend()
        result = worker.handle(self._observe_envelope(), backend, lambda: 100000,
                               lock_path=self.lock_path)["output"]
        self.assertFalse(result["ready"])
        self.assertEqual(result["session"]["reason"], "desktop_lock_unavailable")
        self.assertEqual(target.read_text(), "unchanged")
        self.assertEqual(backend.observations, 0)


class NativeDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.lock_path = pathlib.Path(self.directory.name) / "desktop.lock"
        self.secret = "UI secret marker /private/user/path"

    def tearDown(self):
        self.directory.cleanup()

    def run_main(self, envelope, backend):
        stdin = mock.patch.object(sys, "stdin", io.StringIO(json.dumps(envelope)))
        stdout = mock.patch.object(sys, "stdout", io.StringIO())
        stderr = mock.patch.object(sys, "stderr", io.StringIO())
        with stdin, stdout as out, stderr as err:
            with self.assertRaises(SystemExit) as exit_result:
                worker.main(backend=backend, lock_path=self.lock_path)
        return exit_result.exception.code, out.getvalue(), err.getvalue()

    def observe_envelope(self):
        return {"protocol_version": "0.1", "config": config(),
                "input": {"operation": "observe", "goal": "synthetic"}}

    def test_unexpected_observation_failure_has_allowlisted_diagnostic_only(self):
        secret = self.secret
        class Failing(Backend):
            def observe(self, allowed): raise RuntimeError(secret)
        code, stdout, stderr = self.run_main(self.observe_envelope(), Failing())
        self.assertEqual(code, 71)
        self.assertEqual(stdout, "")
        self.assertEqual(json.loads(stderr), {"stage": "observe", "code": "native_observe_failed", "exit_code": 71})
        self.assertNotIn(self.secret, stderr)

    def test_unexpected_action_validation_failure_does_not_dispatch(self):
        backend = Backend()
        now = worker.now_ms()
        malformed = {"session": {"halted": False, "steps": 0, "started_ms": now},
                     "captured_ms": now, "screen": {}}
        envelope = {"protocol_version": "0.1", "config": config(),
                    "input": {"operation": "act", "observation": malformed,
                              "decision": {"choice": "click_target_0", "confidence": .95}}}
        code, stdout, stderr = self.run_main(envelope, backend)
        self.assertEqual(code, 72)
        self.assertEqual(stdout, "")
        self.assertEqual(backend.actions, [])
        self.assertEqual(json.loads(stderr), {"stage": "action_validation", "code": "native_action_validation_failed", "exit_code": 72})
        self.assertNotIn(self.secret, stderr)

    def test_event_dispatch_failure_is_nonzero_and_redacts_raw_exception(self):
        backend = Backend()
        backend.fail = True
        envelope = self.observe_envelope()
        observation = worker.handle(envelope, Backend(), worker.now_ms,
                                    lock_path=self.lock_path)["output"]
        action = {"protocol_version": "0.1", "config": config(),
                  "input": {"operation": "act", "observation": observation,
                            "decision": {"choice": "click_target_0", "confidence": .95}}}
        code, stdout, stderr = self.run_main(action, backend)
        self.assertEqual(code, 73)
        self.assertEqual(stdout, "")
        self.assertEqual(backend.actions, ["click_target_0"])
        self.assertEqual(json.loads(stderr), {"stage": "event_dispatch", "code": "native_event_dispatch_failed", "exit_code": 73})
        self.assertNotIn(self.secret, stderr)

    def test_post_action_verification_failure_never_returns_session_success(self):
        secret = self.secret
        class FailingAfterAction(Backend):
            def observe(self, allowed):
                self.observations += 1
                if self.observations == 2: raise RuntimeError(secret)
                return copy.deepcopy(self.value)
        backend = FailingAfterAction()
        observation = worker.handle(self.observe_envelope(), Backend(), worker.now_ms,
                                    lock_path=self.lock_path)["output"]
        action = {"protocol_version": "0.1", "config": config(),
                  "input": {"operation": "act", "observation": observation,
                            "decision": {"choice": "click_target_0", "confidence": .95}}}
        code, stdout, stderr = self.run_main(action, backend)
        self.assertEqual(code, 74)
        self.assertEqual(stdout, "")
        self.assertEqual(backend.actions, ["click_target_0"])
        self.assertEqual(json.loads(stderr), {"stage": "post_action_verification", "code": "native_post_action_verification_failed", "exit_code": 74})
        self.assertNotIn(self.secret, stderr)


if __name__ == "__main__":
    unittest.main()
