/// <reference types="solid-js" />
import type { AuthSession } from "@revscope/web-shared/authState";
import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { fetchSiteAdminAccounts, type SiteAdminAccountsPage } from "../../../lib/siteAdminApi";
import { secondaryButton } from "../../buttons.css";
import { errorMessage, message } from "../messages.css";
import PaginationBar from "../PaginationBar";
import { visuallyHidden } from "../visuallyHidden.css";
import SiteAdminAccountRow from "./SiteAdminAccountRow";
import { accountsTable, heading, tableScroll } from "./SiteAdminAccounts.css";

/** The Admin tab: every account on the server, newest first, a page at a time. */
export default function SiteAdminAccounts(props: { session: AuthSession }) {
  const [offset, setOffset] = createSignal(0);
  const [page, { refetch }] = createResource(offset, (from) =>
    fetchSiteAdminAccounts(props.session, from),
  );
  // Reading an errored resource rethrows, so check the error first. The last
  // page that loaded stays up while the next one loads, and if it fails.
  const loaded = createMemo<SiteAdminAccountsPage | undefined>((previous) =>
    page.error ? previous : (page.latest ?? previous),
  );
  const pageLabel = () => {
    const current = loaded();
    if (!current) {
      return "";
    }
    if (current.accounts.length === 0) {
      return `${current.total.toLocaleString()} accounts`;
    }
    const first = current.offset + 1;
    const last = current.offset + current.accounts.length;
    return `${first.toLocaleString()}–${last.toLocaleString()} of ${current.total.toLocaleString()} accounts`;
  };

  // After a failed load the page on screen isn't the one `offset` asks for,
  // so moving to the offset already asked for retries it.
  const goTo = (target: number) => {
    if (target === offset()) {
      void refetch();
    } else {
      setOffset(target);
    }
  };

  // Reloads the page on screen, or the one before it if the deletion emptied it.
  const afterDelete = () => {
    const current = loaded();
    if (!current) {
      return;
    }
    const emptied = current.accounts.length === 1 && current.offset > 0;
    goTo(emptied ? Math.max(0, current.offset - current.page_size) : current.offset);
  };

  return (
    <section>
      <h2 class={heading}>Accounts</h2>
      <Show when={page.error}>
        <p class={errorMessage}>{(page.error as Error).message}</p>
        <button type="button" class={secondaryButton} onClick={() => void refetch()}>
          Try again
        </button>
      </Show>
      <Show
        when={loaded()}
        fallback={
          <Show when={!page.error}>
            <p class={message}>Loading accounts…</p>
          </Show>
        }
      >
        {(current) => (
          <>
            <div class={tableScroll}>
              <table class={accountsTable}>
                <thead>
                  <tr>
                    <th scope="col">Account</th>
                    <th scope="col">Users</th>
                    <th scope="col">Projects</th>
                    <th scope="col">Plan</th>
                    <th scope="col">Subscription</th>
                    <th scope="col">
                      <span class={visuallyHidden}>Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  <For each={current().accounts}>
                    {(account) => (
                      <SiteAdminAccountRow
                        session={props.session}
                        account={account}
                        onDeleted={afterDelete}
                      />
                    )}
                  </For>
                </tbody>
              </table>
            </div>
            <PaginationBar
              label={pageLabel()}
              loading={page.loading}
              atStart={current().offset === 0}
              atEnd={current().offset + current().page_size >= current().total}
              onPrevious={() => goTo(Math.max(0, current().offset - current().page_size))}
              onNext={() => goTo(current().offset + current().page_size)}
              showButtons={current().total > current().page_size}
            />
          </>
        )}
      </Show>
    </section>
  );
}
