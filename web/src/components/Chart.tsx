import { createEffect, onCleanup, onMount } from "solid-js";
import { echarts } from "@/lib/charts/echarts";
import { paletteVersion } from "@/lib/prefs";
import { locale } from "@/i18n";

type Option = Record<string, unknown>;

/**
 * Thin ECharts wrapper. `option` is a tracked accessor: it re-runs when its data, the theme or
 * the locale change. While `busy` the previous render stays visible at reduced opacity.
 */
export function Chart(props: {
  option: () => Option | null;
  height: number | string;
  onClick?: (params: any) => void;
  busy?: boolean;
  class?: string;
  ariaLabel?: string;
}) {
  let el!: HTMLDivElement;
  let chart: echarts.ECharts | undefined;
  let observer: ResizeObserver | undefined;

  onMount(() => {
    chart = echarts.init(el, undefined, { renderer: "canvas" });
    if (props.onClick) chart.on("click", (params: unknown) => props.onClick?.(params));
    observer = new ResizeObserver(() => chart?.resize());
    observer.observe(el);
  });

  createEffect(() => {
    paletteVersion();
    locale();
    const option = props.option();
    if (!chart) return;
    if (option) chart.setOption(option, { notMerge: true, lazyUpdate: true });
    else chart.clear();
  });

  onCleanup(() => {
    observer?.disconnect();
    chart?.dispose();
  });

  return (
    <div
      ref={el}
      class={`${props.class ?? ""} ${props.busy ? "refetching" : ""}`}
      role="img"
      aria-label={props.ariaLabel}
      style={{ width: "100%", height: typeof props.height === "number" ? `${props.height}px` : props.height }}
    />
  );
}
