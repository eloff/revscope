/**
 * Tree-shaken ApexCharts: core plus the chart types and features we use.
 * `import ApexCharts from "apexcharts"` pulls the full catalogue (~500kB+).
 *
 * Chart-type entry points register families, not a single `chart.type`:
 * `line` also registers area; `bar` also registers column and funnel.
 */
import ApexCharts from "apexcharts/core";
import "apexcharts/bar";
import "apexcharts/features/annotations";
import "apexcharts/features/legend";
import "apexcharts/line";
import "apexcharts/treemap";

export type { ApexOptions } from "apexcharts";
export default ApexCharts;
