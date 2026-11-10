/// <reference types="solid-js" />
import { For, type JSX } from "solid-js";
import { setupGuide, setupSteps, setupSummary } from "./Settings.css";

export type GoogleApi = { name: string; id: string };

type Props = {
  /** The APIs the service account's Cloud project must enable. */
  apis: GoogleApi[];
  /** Where to give the service account access in the Google product itself. */
  grant: JSX.Element;
  /** What to do with the key file in RevScope. */
  finish: JSX.Element;
  open: boolean;
};

/** An external link that opens beside the form, so a half-filled form isn't lost. */
export function ExternalLink(props: { href: string; children: JSX.Element }) {
  return (
    <a href={props.href} target="_blank" rel="noreferrer">
      {props.children}
    </a>
  );
}

/**
 * How to create a Google service account and its JSON key. Steps up to the key are
 * the same for every Google integration; the grant and finish steps differ.
 */
export default function ServiceAccountSteps(props: Props) {
  return (
    <details class={setupGuide} open={props.open}>
      <summary class={setupSummary}>How to create a service account</summary>
      <ol class={setupSteps}>
        <li>
          In{" "}
          <ExternalLink href="https://console.cloud.google.com/">Google Cloud Console</ExternalLink>
          , pick or create a project, then enable{" "}
          <For each={props.apis}>
            {(api, index) => (
              <>
                {index() > 0 ? " and " : ""}
                <ExternalLink href={`https://console.cloud.google.com/apis/library/${api.id}`}>
                  {api.name}
                </ExternalLink>
              </>
            )}
          </For>{" "}
          for it.
        </li>
        <li>
          Open{" "}
          <ExternalLink href="https://console.cloud.google.com/iam-admin/serviceaccounts">
            Service accounts
          </ExternalLink>
          , choose <strong>Create service account</strong>, name it, and skip the optional
          permissions steps. It needs no project roles.
        </li>
        <li>
          Open the new service account and choose{" "}
          <strong>Keys → Add key → Create new key → JSON</strong>. A key file downloads. If Google
          refuses because key creation is disabled, an organization admin needs to allow it for this
          project.
        </li>
        <li>{props.grant}</li>
        <li>{props.finish}</li>
      </ol>
    </details>
  );
}
