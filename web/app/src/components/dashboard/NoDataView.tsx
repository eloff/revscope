/// <reference types="solid-js" />
import { appPath, docsUrl } from "@revscope/web-shared/links";
import { Show } from "solid-js";
import { primaryButton, secondaryButton } from "../buttons.css";
import { container, description, heading, links } from "./NoDataView.css";

export type NoDataViewVariant = "date-range" | "payment-activity" | "crawlers";

type NoDataLink = { href: string; label: string };

type NoDataCopy = {
  heading: string;
  description: string;
  primary: NoDataLink;
  secondary?: NoDataLink;
};

type NoDataViewProps = {
  /** Defaults to date-range; payment-activity is for the Revenue tab's fixed window. */
  variant?: NoDataViewVariant;
};

const COPY: Record<NoDataViewVariant, NoDataCopy> = {
  "date-range": {
    heading: "No data for the selected date range",
    description:
      "Try a wider date range. If you're still setting up, add the tracking script to your site and connect the payment provider you bill through.",
    primary: { href: docsUrl("/docs/getting-started"), label: "Getting started" },
    secondary: { href: docsUrl("/docs/integrations"), label: "Payment integrations" },
  },
  "payment-activity": {
    heading: "No payment activity in the last 12 months",
    description:
      "If you're still setting up, add the tracking script to your site and connect the payment provider you bill through.",
    primary: { href: docsUrl("/docs/getting-started"), label: "Getting started" },
    secondary: { href: docsUrl("/docs/integrations"), label: "Payment integrations" },
  },
  crawlers: {
    heading: "No crawler data for the selected date range",
    description:
      "Try a wider date range. Crawler counts come from Cloudflare. Your site's DNS records must be proxied (orange cloud), and you need to connect the Cloudflare integration with an API token.",
    primary: { href: appPath("/settings"), label: "Cloudflare integration" },
  },
};

/** Shown in place of a tab's contents when its query came back empty. */
export default function NoDataView(props: NoDataViewProps) {
  const copy = () => COPY[props.variant ?? "date-range"];

  return (
    <div class={container}>
      <h3 class={heading}>{copy().heading}</h3>
      <p class={description}>{copy().description}</p>
      <div class={links}>
        <a class={primaryButton} href={copy().primary.href}>
          {copy().primary.label}
        </a>
        <Show when={copy().secondary}>
          {(link) => (
            <a class={secondaryButton} href={link().href}>
              {link().label}
            </a>
          )}
        </Show>
      </div>
    </div>
  );
}
