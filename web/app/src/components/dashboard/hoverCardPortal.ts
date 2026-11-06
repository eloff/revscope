import { isServer } from "solid-js/web";
import { HOVER_CARD_LAYER } from "./layers";

let mount: HTMLElement | undefined;

/** Body-level mount so portaled hover cards stack above dashboard layers (e.g. metrics bar). */
export function hoverCardPortalMount(): HTMLElement | undefined {
  if (isServer) {
    return undefined;
  }
  if (!mount) {
    mount = document.createElement("div");
    mount.dataset.hoverCardPortal = "";
    Object.assign(mount.style, {
      position: "relative",
      zIndex: String(HOVER_CARD_LAYER),
    });
    document.body.appendChild(mount);
  }
  return mount;
}
