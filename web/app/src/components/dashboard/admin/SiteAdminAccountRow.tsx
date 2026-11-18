/// <reference types="solid-js" />
import { Menu } from "@ark-ui/solid/menu";
import type { AuthSession } from "@revscope/web-shared/authState";
import { menuContent, menuItem, menuPositioner } from "@revscope/web-shared/menuStyles";
import {
  eventsVolumeLabel,
  formatDate,
  formatMoney,
  planLabel,
} from "@revscope/web-shared/subscriptionLabels";
import { createSignal, For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type {
  SiteAdminAccount,
  SiteAdminSubscription,
  SiteAdminUser,
} from "../../../lib/siteAdminApi";
import { adminTag, pendingTag, youTag } from "../../users/ManageUsers.css";
import DeleteAccountDialog from "./DeleteAccountDialog";
import {
  actionsCell,
  dangerMenuItem,
  menuTrigger,
  monoText,
  primaryText,
  secondaryText,
  stack,
  tagRow,
  unverified,
} from "./SiteAdminAccounts.css";

type SiteAdminAccountRowProps = {
  session: AuthSession;
  account: SiteAdminAccount;
  onDeleted: () => void;
};

const PROVIDER_NAMES: Record<string, string> = { stripe: "Stripe", polar: "Polar" };

function dateLabel(prefix: string, epochSeconds: number | null): string | null {
  return epochSeconds === null ? null : `${prefix} ${formatDate(epochSeconds)}`;
}

/** What the account pays, e.g. "$19.00 / month (list $29.00)". RevScope bills in USD. */
export function priceLabel(subscription: SiteAdminSubscription): string {
  if (subscription.regular_price_cents === 0) {
    return "No charge";
  }
  const period = subscription.billing_period === "yearly" ? "year" : "month";
  const price = `${formatMoney(subscription.actual_price_cents, "usd")} / ${period}`;
  return subscription.actual_price_cents < subscription.regular_price_cents
    ? `${price} (list ${formatMoney(subscription.regular_price_cents, "usd")})`
    : price;
}

/**
 * How the user proved their address, if they have. A linked Google or Apple
 * identity counts even without a date, as it does for confirming the account.
 */
export function verificationLabel(user: SiteAdminUser): string | null {
  if (user.email_verified_at !== null) {
    return `Verified ${formatDate(user.email_verified_at)}`;
  }
  if (user.sign_in_providers.length > 0) {
    const providers = user.sign_in_providers.map(
      (provider) => provider.charAt(0).toUpperCase() + provider.slice(1),
    );
    return `Verified by ${providers.join(" and ")} sign-in`;
  }
  return null;
}

function UserCell(props: { user: SiteAdminUser }) {
  const joined = () =>
    dateLabel(props.user.invited ? "Invited" : "Signed up", props.user.signed_up_at);
  const lastLogin = () => dateLabel("Last login", props.user.last_login_at);
  return (
    <li>
      <span class={tagRow}>
        <span class={primaryText}>{props.user.name}</span>
        <Show when={props.user.is_admin}>
          <span class={adminTag}>Admin</span>
        </Show>
        <Show when={props.user.pending}>
          <span class={pendingTag}>Invited</span>
        </Show>
      </span>
      <span class={secondaryText}>{props.user.email}</span>
      <Show when={joined()}>
        <span class={secondaryText}>{joined()}</span>
      </Show>
      <span class={secondaryText}>
        {verificationLabel(props.user) ?? <span class={unverified}>Email not verified</span>}
      </span>
      <Show when={lastLogin()}>
        <span class={secondaryText}>{lastLogin()}</span>
      </Show>
    </li>
  );
}

function PlanCell(props: { subscription: SiteAdminSubscription }) {
  const discountEnds = () => dateLabel("Discount ends", props.subscription.discount_ends_at);
  const trialEnds = () => dateLabel("Trial ends", props.subscription.trial_ends_at);
  return (
    <>
      <span class={primaryText}>{planLabel(props.subscription.plan)}</span>
      <span class={secondaryText}>{priceLabel(props.subscription)}</span>
      <span class={secondaryText}>
        {eventsVolumeLabel(props.subscription.events_monthly_limit)}
      </span>
      <span class={secondaryText}>Since {formatDate(props.subscription.started_at)}</span>
      <Show when={discountEnds()}>
        <span class={secondaryText}>{discountEnds()}</span>
      </Show>
      <Show when={trialEnds()}>
        <span class={secondaryText}>{trialEnds()}</span>
      </Show>
    </>
  );
}

function SubscriptionCell(props: { subscription: SiteAdminSubscription }) {
  const provider = () => props.subscription.provider;
  const periodEnds = () => dateLabel("Paid through", props.subscription.current_period_end);
  return (
    <Show when={provider()} fallback={<span class={secondaryText}>Not billed</span>}>
      {(name) => (
        <>
          <span class={primaryText}>
            {PROVIDER_NAMES[name()] ?? name()} · {props.subscription.status ?? "no status yet"}
          </span>
          <Show when={periodEnds()}>
            <span class={secondaryText}>{periodEnds()}</span>
          </Show>
          <Show when={props.subscription.subscription_id}>
            <span class={monoText}>{props.subscription.subscription_id}</span>
          </Show>
          <Show when={props.subscription.customer_id}>
            <span class={monoText}>{props.subscription.customer_id}</span>
          </Show>
        </>
      )}
    </Show>
  );
}

export default function SiteAdminAccountRow(props: SiteAdminAccountRowProps) {
  const [confirmingDelete, setConfirmingDelete] = createSignal(false);
  const signedUp = () => dateLabel("Signed up", props.account.signed_up_at);

  return (
    <tr aria-label={props.account.name}>
      <td>
        <span class={tagRow}>
          <span class={primaryText}>{props.account.name}</span>
          <Show when={props.account.is_own}>
            <span class={youTag}>You</span>
          </Show>
        </span>
        <span class={monoText}>{props.account.account_id}</span>
        <Show when={signedUp()}>
          <span class={secondaryText}>{signedUp()}</span>
        </Show>
      </td>
      <td>
        <ul class={stack}>
          <For each={props.account.users} fallback={<li class={secondaryText}>No users</li>}>
            {(user) => <UserCell user={user} />}
          </For>
        </ul>
      </td>
      <td>
        <ul class={stack}>
          <For each={props.account.projects} fallback={<li class={secondaryText}>No projects</li>}>
            {(project) => (
              <li>
                <span class={primaryText}>{project.name}</span>
                <span class={secondaryText}>ID {project.id}</span>
              </li>
            )}
          </For>
        </ul>
      </td>
      <td>
        <Show when={props.account.subscription} fallback={<span class={secondaryText}>None</span>}>
          {(subscription) => <PlanCell subscription={subscription()} />}
        </Show>
      </td>
      <td>
        <Show when={props.account.subscription} fallback={<span class={secondaryText}>None</span>}>
          {(subscription) => <SubscriptionCell subscription={subscription()} />}
        </Show>
      </td>
      <td class={actionsCell}>
        <Menu.Root positioning={{ placement: "bottom-end", gutter: 4 }}>
          <Menu.Trigger
            class={menuTrigger}
            aria-label={`Actions for ${props.account.name}`}
            disabled={props.account.is_own}
            title={props.account.is_own ? "This is your own account" : undefined}
          >
            ⋯
          </Menu.Trigger>
          <Portal>
            <Menu.Positioner class={menuPositioner}>
              <Menu.Content class={menuContent}>
                <Menu.Item
                  value="delete"
                  class={`${menuItem} ${dangerMenuItem}`}
                  onSelect={() => setConfirmingDelete(true)}
                >
                  Delete account…
                </Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Portal>
        </Menu.Root>
        <DeleteAccountDialog
          session={props.session}
          account={props.account}
          open={confirmingDelete()}
          onClose={() => setConfirmingDelete(false)}
          onDeleted={props.onDeleted}
        />
      </td>
    </tr>
  );
}
