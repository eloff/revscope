/**
 * Reads a dashboard cell or tile as a number: `$1.9k`, `$1.20M`, `10,244`,
 * `63.0%`, `1h 2m`, with or without the ★ that marks a column's best value. `—`
 * means no value. Anything else throws, so a format change fails loudly rather
 * than quietly skewing a comparison.
 */
export function parseValue(raw: string): number | null {
  const text = raw.replace("★", "").trim();
  if (text === "—") {
    return null;
  }
  const number = /^(-?)\$?([\d,]+(?:\.\d+)?)\s*([kKM]?)\s*%?$/.exec(text);
  if (number) {
    const scale = { "": 1, k: 1_000, K: 1_000, M: 1_000_000 }[number[3]] ?? 1;
    const magnitude = Number(number[2].replaceAll(",", "")) * scale;
    return number[1] ? -magnitude : magnitude;
  }
  const duration = /^(?:(\d+)h)?\s*(?:(\d+)m)?\s*(?:(\d+)s)?$/.exec(text);
  if (duration && text !== "") {
    const [hours, minutes, seconds] = duration.slice(1).map((part) => Number(part ?? 0));
    return hours * 3600 + minutes * 60 + seconds;
  }
  throw new Error(`Unrecognized dashboard value: ${JSON.stringify(raw)}`);
}

/** The date pickers' `mm/dd/yyyy` as a sortable timestamp. */
export function parsePickerDate(text: string): number {
  const match = /^(\d{2})\/(\d{2})\/(\d{4})$/.exec(text);
  if (!match) {
    throw new Error(`Unrecognized picker date: ${JSON.stringify(text)}`);
  }
  return Date.UTC(Number(match[3]), Number(match[1]) - 1, Number(match[2]));
}
