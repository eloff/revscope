/// <reference types="solid-js" />
import { createEffect, onCleanup, onMount } from "solid-js";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import { sparklineMount } from "./Sparkline.css";

type SparklineProps = {
  values: number[];
  categories?: string[];
  color?: string;
  height?: number;
};

const DEFAULT_COLOR = "#0891b2";
const DEFAULT_HEIGHT = 40;

function buildOptions(
  values: number[],
  categories: string[],
  color: string,
  height: number,
): ApexOptions {
  return {
    chart: {
      type: "area",
      height,
      width: "100%",
      sparkline: { enabled: true },
      animations: { enabled: false },
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
    },
    stroke: {
      width: 2,
      curve: "smooth",
    },
    fill: {
      type: "solid",
      opacity: 0.22,
    },
    colors: [color],
    dataLabels: { enabled: false },
    grid: { show: false },
    series: [{ name: "Views", data: values }],
    xaxis: {
      categories,
      labels: { show: false },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      min: 0,
      show: false,
      labels: { show: false },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    tooltip: {
      enabled: true,
      y: {
        formatter: (value) => `${Math.round(value).toLocaleString()} views`,
        title: {
          formatter: () => "",
        },
      },
    },
  };
}

export default function Sparkline(props: SparklineProps) {
  let mount: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;

  const color = () => props.color ?? DEFAULT_COLOR;
  const height = () => props.height ?? DEFAULT_HEIGHT;
  const categories = () => props.categories ?? props.values.map((_, index) => String(index + 1));

  onMount(() => {
    if (!mount) {
      return;
    }
    chart = new ApexCharts(mount, {
      chart: { type: "area", height: height(), sparkline: { enabled: true } },
      series: [],
    });
    chart.render();
  });

  createEffect(() => {
    if (!chart) {
      return;
    }
    chart.updateOptions(buildOptions(props.values, categories(), color(), height()), true, true);
  });

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  return <div ref={mount} class={sparklineMount} />;
}
