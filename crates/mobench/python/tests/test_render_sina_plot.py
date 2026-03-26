import importlib.util
import json
import math
import pathlib
import sys
import tempfile
import types
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


class FakeAxes:
    def __init__(self):
        self.scatter_calls = []
        self.hline_calls = []
        self.title = None
        self.xlabel = None
        self.ylabel = None
        self.xticks = None
        self.xticklabels = None
        self.xlim = None
        self.margins_args = None
        self.ylim = None

    def scatter(self, x, y, **kwargs):
        self.scatter_calls.append((list(x), list(y), kwargs))

    def hlines(self, y, xmin, xmax, **kwargs):
        self.hline_calls.append((y, xmin, xmax, kwargs))

    def set_title(self, value):
        self.title = value

    def set_xlabel(self, value):
        self.xlabel = value

    def set_ylabel(self, value):
        self.ylabel = value

    def set_xticks(self, values):
        self.xticks = list(values)

    def set_xticklabels(self, values, **kwargs):
        self.xticklabels = list(values)

    def set_xlim(self, left, right):
        self.xlim = (left, right)

    def margins(self, **kwargs):
        self.margins_args = kwargs

    def set_ylim(self, bottom, top):
        self.ylim = (bottom, top)


class FakeFigure:
    def __init__(self):
        self.saved = None

    def savefig(self, path, **kwargs):
        self.saved = (path, kwargs)


class FakeStyle:
    def __init__(self):
        self.paths = []

    def use(self, path):
        self.paths.append(path)


class FakePyplot(types.SimpleNamespace):
    def __init__(self):
        super().__init__()
        self.style = FakeStyle()
        self.figure = FakeFigure()
        self.axes = FakeAxes()

    def subplots(self, figsize=None):
        self.figsize = figsize
        return self.figure, self.axes

    def get_cmap(self, name):
        self.cmap_name = name

        def color(idx):
            return f"color-{idx}"

        return color

    def close(self, fig):
        self.closed = fig


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


class LayoutTests(unittest.TestCase):
    def test_render_plot_expands_dense_columns_without_overlap(self):
        mod = load_module()
        dense_samples = [10_000_000] * 30
        spec = {
            "function_name": "nullifier-proof-generation",
            "function_label": "Nullifier proof generation",
            "target": "benchmark-1",
            "devices": [
                {
                    "device_name": "iPhone 15",
                    "os_version": "iOS 17.4",
                    "samples_ns": dense_samples,
                },
                {
                    "device_name": "Pixel 8",
                    "os_version": "Android 15",
                    "samples_ns": dense_samples,
                },
            ],
        }

        fake_pyplot = FakePyplot()
        fake_matplotlib = types.ModuleType("matplotlib")
        fake_matplotlib.__path__ = []
        fake_matplotlib.use = lambda backend: None
        with mock.patch.dict(
            sys.modules,
            {
                "matplotlib": fake_matplotlib,
                "matplotlib.pyplot": fake_pyplot,
            },
        ):
            mod.render_plot(spec, pathlib.Path("/tmp/out.svg"))

        self.assertEqual(len(fake_pyplot.axes.scatter_calls), 2)
        first_xs = fake_pyplot.axes.scatter_calls[0][0]
        second_xs = fake_pyplot.axes.scatter_calls[1][0]
        self.assertLess(max(first_xs), min(second_xs))

    def test_render_plot_centers_median_marker_on_adaptive_device_center(self):
        mod = load_module()
        dense_samples = [10_000_000] * 30
        spec = {
            "function_name": "nullifier-proof-generation",
            "function_label": "Nullifier proof generation",
            "target": "benchmark-1",
            "devices": [
                {
                    "device_name": "iPhone 15",
                    "os_version": "iOS 17.4",
                    "samples_ns": dense_samples,
                },
                {
                    "device_name": "Pixel 8",
                    "os_version": "Android 15",
                    "samples_ns": dense_samples,
                },
            ],
        }

        fake_pyplot = FakePyplot()
        fake_matplotlib = types.ModuleType("matplotlib")
        fake_matplotlib.__path__ = []
        fake_matplotlib.use = lambda backend: None
        with mock.patch.dict(
            sys.modules,
            {
                "matplotlib": fake_matplotlib,
                "matplotlib.pyplot": fake_pyplot,
            },
        ):
            mod.render_plot(spec, pathlib.Path("/tmp/out.svg"))

        self.assertEqual(len(fake_pyplot.axes.scatter_calls), 2)
        self.assertEqual(len(fake_pyplot.axes.hline_calls), 2)
        for scatter_call, hline_call in zip(
            fake_pyplot.axes.scatter_calls, fake_pyplot.axes.hline_calls
        ):
            xs, _, _ = scatter_call
            _, xmin, xmax, _ = hline_call
            expected_center = sum(xs) / len(xs)
            self.assertAlmostEqual((xmin + xmax) / 2.0, expected_center, places=6)


@unittest.skipIf(importlib.util.find_spec("matplotlib") is None, "matplotlib not installed")
class MatplotlibSmokeTests(unittest.TestCase):
    def test_render_plot_writes_svg(self):
        mod = load_module()
        spec = {
            "function_name": "nullifier-proof-generation",
            "function_label": "Nullifier proof generation",
            "target": "benchmark-1",
            "devices": [
                {
                    "device_name": "iPhone 15",
                    "os_version": "iOS 17.4",
                    "samples_ns": [10_000_000, 10_100_000, 10_200_000],
                }
            ],
        }

        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = pathlib.Path(tmpdir) / "plot.svg"
            mod.render_plot(spec, output_path)
            self.assertTrue(output_path.exists())
            svg = output_path.read_text(encoding="utf-8")
            self.assertIn("<svg", svg)
