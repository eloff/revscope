/** A left click without modifier keys, which a link handler may take over; the rest open new tabs or windows. */
export function isPlainLeftClick(event: MouseEvent): boolean {
  return event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
}
