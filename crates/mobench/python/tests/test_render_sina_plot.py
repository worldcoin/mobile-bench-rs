import importlib.util
import math
import pathlib
import unittest


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

