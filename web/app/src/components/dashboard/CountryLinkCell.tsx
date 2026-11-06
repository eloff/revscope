/// <reference types="solid-js" />
import { type JSX, Show } from "solid-js";
import { countryDisplayName, isIsoCountryCode } from "./audienceLogic";
import CountryFlag from "./CountryFlag";
import { countryLinkButton, countryLinkButtonPill } from "./CountryLinkCell.css";
import { gridIconLabelCell, gridInlineActionLabel } from "./grid.css";
import { gridInlineActionIcon } from "./icons.css";

export type CountryLinkCellProps = {
  country?: string;
  /** Display text; defaults to the country name when country is a valid ISO code. */
  label?: string;
  onCountrySelect?: (countryCode: string) => void;
};

export function visitorLocationLabel(country?: string, city?: string): string | undefined {
  if (!city && !country) {
    return undefined;
  }
  const countryCode = country?.toUpperCase();
  const countryName =
    country && countryCode && isIsoCountryCode(countryCode)
      ? countryDisplayName(countryCode)
      : country;
  if (city && countryName) {
    return `${city}, ${countryName}`;
  }
  return city ?? countryName;
}

export default function CountryLinkCell(props: CountryLinkCellProps) {
  const countryCode = () => props.country?.toUpperCase();
  const displayLabel = () => {
    if (props.label) {
      return props.label;
    }
    const code = countryCode();
    if (code && isIsoCountryCode(code)) {
      return countryDisplayName(code);
    }
    return props.country ?? "—";
  };
  const canLink = () => {
    const code = countryCode();
    return props.onCountrySelect !== undefined && code !== undefined && isIsoCountryCode(code);
  };

  const countryFlag = (): JSX.Element | undefined => {
    const code = countryCode();
    if (!code || !isIsoCountryCode(code)) {
      return undefined;
    }
    return <CountryFlag code={code} />;
  };

  const content = (): JSX.Element => {
    const flag = countryFlag();
    if (flag) {
      return (
        <span class={gridIconLabelCell}>
          {flag}
          <span>{displayLabel()}</span>
        </span>
      );
    }
    return <>{displayLabel()}</>;
  };

  const linkTitle = () => {
    const code = countryCode();
    return code ? `View details for ${countryDisplayName(code)} in Audience` : "";
  };

  return (
    <Show when={canLink()} fallback={content()}>
      <button
        type="button"
        class={countryLinkButton}
        title={linkTitle()}
        aria-label={linkTitle()}
        onClick={(event) => {
          event.stopPropagation();
          const code = countryCode();
          if (code) {
            props.onCountrySelect?.(code);
          }
        }}
      >
        {countryFlag()}
        <span class={countryLinkButtonPill}>
          <span class={gridInlineActionLabel}>{displayLabel()}</span>
          <span class={gridInlineActionIcon} aria-hidden="true" />
        </span>
      </button>
    </Show>
  );
}
