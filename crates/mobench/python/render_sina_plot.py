from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Iterable, Sequence


def pack_strip(
    ys: Sequence[float],
    epsilon: float,
    step: float,
    max_width: float,
) -> list[float]:
    """Pack a vertical strip of points with deterministic horizontal offsets.

    The input order is preserved. The offsets are centered at the end so the
    strip stays visually balanced around x = 0.
    """

    placed: list[tuple[float, float]] = []

    for y in ys:
        offset = _find_offset_for_point(y, placed, epsilon, step, max_width)
        placed.append((offset, y))

    if not placed:
        return []

    mean_x = sum(x for x, _ in placed) / len(placed)
    return [x - mean_x for x, _ in placed]


def _find_offset_for_point(
    y: float,
    placed: Sequence[tuple[float, float]],
    epsilon: float,
    step: float,
    max_width: float,
) -> float:
    if step <= 0:
        raise ValueError("step must be positive")
    if max_width < 0:
        raise ValueError("max_width must be non-negative")

    max_ring = max(1, math.ceil(max_width / step))

    ring = 0
    while ring <= max_ring:
        candidates = [0.0] if ring == 0 else [ring * step, -ring * step]
        for dx in candidates:
            if abs(dx) > max_width:
                continue
            if not placed:
                return dx

            if all((dx - ox) ** 2 + (y - oy) ** 2 >= epsilon**2 for ox, oy in placed):
                return dx
        ring += 1

    raise ValueError("unable to place point within max_width while preserving epsilon")


def render_plot(spec: dict[str, object], output_path: Path) -> None:
    plot = _normalize_plot_spec(spec)
    _validate_plot_spec(plot)

    matplotlib = _import_matplotlib()
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    style_path = Path(__file__).with_name("mobench_light.mplstyle")
    plt.style.use(str(style_path))

    devices = plot["devices"]
    all_samples_ms = [
        sample / 1_000_000.0
        for device in devices
        for sample in device["samples_ns"]
    ]
    if not all_samples_ms:
        raise ValueError("plot spec must contain at least one sample")

    y_min = min(all_samples_ms)
    y_max = max(all_samples_ms)
    y_span = max(y_max - y_min, 1e-9)

    fig_width = max(6.0, 1.2 * len(devices) + 1.6)
    fig, ax = plt.subplots(figsize=(fig_width, 4.8))
    colors = plt.get_cmap("tab10")

    for idx, device in enumerate(devices):
        samples_ms = [sample / 1_000_000.0 for sample in device["samples_ns"]]
        normalized = [(sample - y_min) / y_span for sample in samples_ms]
        offsets = pack_strip(
            normalized,
            epsilon=0.08,
            step=0.02,
            max_width=0.28,
        )
        x_positions = [idx + dx for dx in offsets]
        color = colors(idx % 10)

        ax.scatter(
            x_positions,
            samples_ms,
            s=18,
            color=color,
            alpha=0.8,
            edgecolors="none",
            rasterized=False,
        )

        median_ms = _median(samples_ms)
        ax.hlines(
            median_ms,
            idx - 0.22,
            idx + 0.22,
            color=color,
            linewidth=1.4,
            alpha=0.85,
        )

    title = plot["function_label"]
    target = plot["target"]
    if target:
        title = f"{title} on {target}"
    ax.set_title(title)
    ax.set_xlabel("Device")
    ax.set_ylabel("Runtime (ms)")
    ax.set_xticks(range(len(devices)))
    ax.set_xticklabels(
        [
            _format_device_label(device["device_name"], device["os_version"])
            for device in devices
        ],
        rotation=20,
        ha="right",
    )
    ax.set_xlim(-0.6, len(devices) - 0.4)
    ax.margins(x=0.02, y=0.08)

    # Keep the y-axis tight to the data while leaving room for the points.
    pad = max((y_max - y_min) * 0.08, 0.03)
    ax.set_ylim(y_min - pad, y_max + pad)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, format="svg", bbox_inches="tight")
    plt.close(fig)


def _normalize_plot_spec(spec: dict[str, object]) -> dict[str, object]:
    if "function_name" in spec:
        return spec
    if "plot" in spec and isinstance(spec["plot"], dict):
        return spec["plot"]
    raise ValueError("expected a single plot specification")


def _validate_plot_spec(plot: dict[str, object]) -> None:
    devices = plot.get("devices")
    if not isinstance(devices, list) or not devices:
        raise ValueError("plot spec must contain at least one device")

    for device in devices:
        samples_ns = device.get("samples_ns") if isinstance(device, dict) else None
        if not isinstance(samples_ns, list) or not samples_ns:
            raise ValueError("each device must contain at least one sample")


def _format_device_label(device_name: str, os_version: str) -> str:
    return device_name if not os_version else f"{device_name} {os_version}"


def _median(values: Sequence[float]) -> float:
    ordered = sorted(values)
    n = len(ordered)
    mid = n // 2
    if n % 2 == 1:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2.0


def _import_matplotlib():
    import matplotlib

    return matplotlib


def _load_json(path: Path) -> dict[str, object]:
    with path.open("r", encoding="utf-8") as f:
        payload = json.load(f)
    if not isinstance(payload, dict):
        raise ValueError("plot input must be a JSON object")
    return payload


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Render a sina-style device comparison plot")
    parser.add_argument("--input", required=True, help="Path to normalized plot JSON")
    parser.add_argument("--output", required=True, help="Path to write the SVG plot")
    args = parser.parse_args(list(argv) if argv is not None else None)

    spec = _load_json(Path(args.input))
    render_plot(spec, Path(args.output))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
