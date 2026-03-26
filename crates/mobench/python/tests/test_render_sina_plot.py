import importlib.util
import json
import math
import pathlib
import tempfile
import unittest
from unittest import mock


def load_module():
    path = pathlib.Path(__file__).resolve().parents[1] / "render_sina_plot.py"
    spec = importlib.util.spec_from_file_location("render_sina_plot", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class PackStripTests(unittest.TestCase):
    def test_pack_strip_is_deterministic_and_centered(self):
        mod = load_module()
        xs = mod.pack_strip([10.0, 10.0, 10.0, 10.2], epsilon=0.12, step=0.02, max_width=0.4)
        self.assertEqual(
            xs,
            mod.pack_strip([10.0, 10.0, 10.0, 10.2], epsilon=0.12, step=0.02, max_width=0.4),
        )
        self.assertAlmostEqual(sum(xs), 0.0, places=6)

    def test_pack_strip_respects_minimum_distance(self):
        mod = load_module()
        ys = [1.0, 1.0, 1.05, 1.07, 1.10]
        xs = mod.pack_strip(ys, epsilon=0.10, step=0.01, max_width=0.4)
        for i, (x1, y1) in enumerate(zip(xs, ys)):
            for x2, y2 in zip(xs[i + 1 :], ys[i + 1 :]):
                self.assertGreaterEqual(math.hypot(x2 - x1, y2 - y1), 0.10 - 1e-9)

    def test_pack_strip_raises_when_strip_is_saturated(self):
        mod = load_module()
        with self.assertRaises(ValueError):
            mod.pack_strip([0.0, 0.0], epsilon=0.20, step=0.05, max_width=0.10)


class CliTests(unittest.TestCase):
    def test_main_dispatches_input_json_to_renderer(self):
        mod = load_module()
        payload = {
            "function_name": "nullifier-proof-generation",
            "function_label": "Nullifier proof generation",
            "target": "benchmark-1",
            "devices": [
                {
                    "device_name": "iPhone 15",
                    "os_version": "iOS 17.4",
                    "samples_ns": [10_000_000, 11_000_000],
                }
            ],
        }

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir_path = pathlib.Path(tmpdir)
            input_path = tmpdir_path / "input.json"
            output_path = tmpdir_path / "output.svg"
            input_path.write_text(json.dumps(payload), encoding="utf-8")

            with mock.patch.object(mod, "render_plot") as render_plot:
                exit_code = mod.main([
                    "--input",
                    str(input_path),
                    "--output",
                    str(output_path),
                ])

        self.assertEqual(exit_code, 0)
        render_plot.assert_called_once()
        called_spec, called_output = render_plot.call_args.args
        self.assertEqual(called_spec, payload)
        self.assertEqual(called_output, output_path)


class ValidationTests(unittest.TestCase):
    def test_render_plot_rejects_device_with_empty_samples(self):
        mod = load_module()
        spec = {
            "function_name": "nullifier-proof-generation",
            "function_label": "Nullifier proof generation",
            "target": "benchmark-1",
            "devices": [
                {
                    "device_name": "iPhone 15",
                    "os_version": "iOS 17.4",
                    "samples_ns": [10_000_000, 11_000_000],
                },
                {
                    "device_name": "Pixel 8",
                    "os_version": "Android 15",
                    "samples_ns": [],
                },
            ],
        }

        with self.assertRaises(ValueError):
            mod.render_plot(spec, pathlib.Path("/tmp/out.svg"))
