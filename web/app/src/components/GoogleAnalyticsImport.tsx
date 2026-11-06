/// <reference types="solid-js" />
import { createListCollection, Select } from "@ark-ui/solid/select";
import type { AuthSession } from "@revscope/web-shared/authState";
import { docsUrl } from "@revscope/web-shared/links";
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import {
  cancelGaImport,
  connectServiceAccount,
  deleteGaImport,
  fetchGaImports,
  fetchGaOauthUrl,
  type GaConnection,
  type GaImport,
  type GaImportsResponse,
  overlapsTracking,
  saveImportedDataSetting,
  startGaImport,
  takePendingConnection,
} from "../lib/gaImportApi";
import {
  connectOptions,
  dateRow,
  form,
  importActions,
  importDetail,
  importError,
  importList,
  importName,
  importRow,
  keyInput,
  toggleRow,
  warning,
} from "./GoogleAnalyticsImport.css";
import { content as selectContent, item as selectItem } from "./ProjectSelect.css";
import {
  field,
  fieldHelp,
  fieldLabel,
  inlineButton,
  input,
  saveButton,
  section,
  sectionDescription,
  sectionTitle,
  selectIndicator,
  selectPositioner,
  selectTrigger,
  statusError,
  statusMessage,
} from "./Settings.css";

const POLL_INTERVAL_MS = 5000;
const DOCS_PATH = "/docs/integrations/google-analytics";

type Props = {
  session: AuthSession;
  projectId: string;
};

function isActive(item: GaImport): boolean {
  return item.status === "running" || item.status === "paused";
}

function statusText(item: GaImport): string {
  const percent =
    item.chunks_total > 0 ? Math.floor((item.chunks_done / item.chunks_total) * 100) : 0;
  switch (item.status) {
    case "running":
      return item.error ?? `Importing, ${percent}% done`;
    case "paused":
      return `Paused at ${percent}%: out of Google Analytics quota, resumes automatically`;
    case "completed":
      return "Finished";
    case "cancelled":
      return `Cancelled at ${percent}%`;
    case "failed":
      return `Failed at ${percent}%`;
  }
}

export default function GoogleAnalyticsImport(props: Props) {
  const [data, setData] = createSignal<GaImportsResponse>();
  const [connection, setConnection] = createSignal<GaConnection | null>(null);
  const [serviceAccountJson, setServiceAccountJson] = createSignal("");
  const [propertyId, setPropertyId] = createSignal("");
  const [startDate, setStartDate] = createSignal("");
  const [endDate, setEndDate] = createSignal("");
  const [hostnames, setHostnames] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [confirmingDelete, setConfirmingDelete] = createSignal<number | null>(null);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Something went wrong.");
    } finally {
      setBusy(false);
    }
  };

  const refresh = async () => {
    const response = await fetchGaImports(props.session, props.projectId);
    setData(response);
    return response;
  };

  const useConnection = (connected: GaConnection) => {
    setConnection(connected);
    setPropertyId(connected.properties[0]?.id ?? "");
    setHostnames(connected.known_hostnames.join(", "));
  };

  let poll: ReturnType<typeof setInterval> | undefined;
  onMount(() => {
    const pending = takePendingConnection(props.projectId);
    if (pending) {
      useConnection(pending);
    }
    void run(async () => {
      const response = await refresh();
      setStartDate(response.default_start_date);
      setEndDate(response.default_end_date);
    });
    poll = setInterval(() => {
      if (data()?.imports.some(isActive)) {
        void refresh().catch(() => {});
      }
    }, POLL_INTERVAL_MS);
  });
  onCleanup(() => clearInterval(poll));

  const properties = createMemo(() =>
    createListCollection({
      items: (connection()?.properties ?? []).map((property) => ({
        value: property.id,
        label: `${property.name} (${property.account})`,
      })),
    }),
  );
  const duplicates = () => overlapsTracking(endDate(), data()?.first_tracked_date ?? null);
  const startTooOld = () => {
    const oldest = data()?.oldest_allowed_date;
    return Boolean(oldest && startDate() && startDate() < oldest);
  };
  /** Why these dates can't start, mirroring the server's checks so it needn't refuse them. */
  const rangeProblem = () => {
    const latest = data()?.latest_allowed_date;
    if (!startDate()) {
      return "Choose a start date.";
    }
    if (!endDate()) {
      return "Choose an end date.";
    }
    if (startTooOld()) {
      return "That start date is older than your plan keeps.";
    }
    if (latest && endDate() > latest) {
      return "The end date must be before today, since today's numbers aren't final.";
    }
    return endDate() < startDate() ? "The end date is before the start date." : "";
  };
  // Tracking covers everything the plan keeps, so the defaults are left out of order
  // and there's no choice to correct yet, only one to make.
  const nothingToImport = () => {
    const loaded = data();
    return (
      loaded !== undefined &&
      loaded.default_end_date < loaded.default_start_date &&
      endDate() === loaded.default_end_date &&
      endDate() < startDate() &&
      !startTooOld()
    );
  };
  const running = () => data()?.imports.some(isActive) ?? false;

  const connectWithGoogle = () =>
    run(async () => {
      window.location.assign(await fetchGaOauthUrl(props.session, props.projectId));
    });

  const connectWithKey = () =>
    run(async () => {
      useConnection(
        await connectServiceAccount(props.session, props.projectId, serviceAccountJson()),
      );
      setServiceAccountJson("");
    });

  const start = () =>
    run(async () => {
      const connected = connection();
      if (!connected) {
        return;
      }
      await startGaImport(props.session, props.projectId, {
        connection_id: connected.connection_id,
        property_id: propertyId(),
        start_date: startDate(),
        end_date: endDate(),
        hostnames: hostnames()
          .split(",")
          .map((hostname) => hostname.trim())
          .filter(Boolean),
        allow_duplicates: duplicates(),
      });
      setConnection(null);
      await refresh();
    });

  const cancel = (id: number) =>
    run(async () => {
      await cancelGaImport(props.session, props.projectId, id);
      await refresh();
    });

  const remove = (id: number) =>
    run(async () => {
      await deleteGaImport(props.session, props.projectId, id);
      setConfirmingDelete(null);
      await refresh();
    });

  const setIncluded = (include: boolean) =>
    run(async () => {
      await saveImportedDataSetting(props.session, props.projectId, include);
      await refresh();
    });

  return (
    <section class={section} id="google-analytics-import">
      <h2 class={sectionTitle}>Import from Google Analytics</h2>
      <p class={sectionDescription}>
        Bring your GA4 history in as daily totals, so your charts start before you installed
        RevScope. <a href={docsUrl(DOCS_PATH)}>How imports work</a>
      </p>

      <Show when={error()}>
        <p class={`${statusMessage} ${statusError}`}>{error()}</p>
      </Show>

      <Show when={data()}>
        {(loaded) => (
          <>
            <Show
              when={connection()}
              fallback={
                <Show when={!running()}>
                  <div class={connectOptions}>
                    <Show when={loaded().oauth_available}>
                      <div>
                        <button
                          type="button"
                          class={saveButton}
                          disabled={busy()}
                          onClick={() => void connectWithGoogle()}
                        >
                          Connect with Google
                        </button>
                      </div>
                    </Show>
                    <div class={field}>
                      <label class={fieldLabel} for="ga-service-account">
                        Service account JSON key
                      </label>
                      <textarea
                        id="ga-service-account"
                        class={keyInput}
                        value={serviceAccountJson()}
                        onInput={(event) => setServiceAccountJson(event.currentTarget.value)}
                      />
                      <p class={fieldHelp}>
                        Add the service account's email as a Viewer on your GA4 property first.
                      </p>
                      <div>
                        <button
                          type="button"
                          class={inlineButton}
                          disabled={busy() || !serviceAccountJson().trim()}
                          onClick={() => void connectWithKey()}
                        >
                          Connect service account
                        </button>
                      </div>
                    </div>
                  </div>
                </Show>
              }
            >
              {(connected) => (
                <Show
                  when={connected().properties.length > 0}
                  fallback={
                    <div class={form}>
                      <p class={`${statusMessage} ${statusError}`}>
                        This credential can't read any GA4 properties. Use a Google account with
                        access to the property, or add the service account's email as a Viewer on it
                        in Google Analytics, then connect again.
                      </p>
                      <div>
                        <button
                          type="button"
                          class={inlineButton}
                          onClick={() => setConnection(null)}
                        >
                          Connect again
                        </button>
                      </div>
                    </div>
                  }
                >
                  <div class={form}>
                    <div class={field}>
                      <span class={fieldLabel} id="ga-property-label">
                        Property
                      </span>
                      <Select.Root
                        collection={properties()}
                        value={propertyId() ? [propertyId()] : []}
                        onValueChange={(details) => setPropertyId(details.value[0] ?? "")}
                      >
                        <Select.Control>
                          <Select.Trigger class={selectTrigger} aria-labelledby="ga-property-label">
                            <Select.ValueText placeholder="Select a property" />
                            <Select.Indicator class={selectIndicator} aria-hidden="true">
                              ▾
                            </Select.Indicator>
                          </Select.Trigger>
                        </Select.Control>
                        <Portal>
                          <Select.Positioner class={selectPositioner}>
                            <Select.Content class={selectContent}>
                              <For each={properties().items}>
                                {(item) => (
                                  <Select.Item item={item} class={selectItem}>
                                    <Select.ItemText>{item.label}</Select.ItemText>
                                  </Select.Item>
                                )}
                              </For>
                            </Select.Content>
                          </Select.Positioner>
                        </Portal>
                      </Select.Root>
                    </div>
                    <div class={dateRow}>
                      <div class={field}>
                        <label class={fieldLabel} for="ga-start-date">
                          From
                        </label>
                        <input
                          id="ga-start-date"
                          class={input}
                          type="date"
                          min={loaded().oldest_allowed_date ?? undefined}
                          max={loaded().latest_allowed_date}
                          value={startDate()}
                          onInput={(event) => setStartDate(event.currentTarget.value)}
                        />
                      </div>
                      <div class={field}>
                        <label class={fieldLabel} for="ga-end-date">
                          To
                        </label>
                        <input
                          id="ga-end-date"
                          class={input}
                          type="date"
                          min={loaded().oldest_allowed_date ?? undefined}
                          max={loaded().latest_allowed_date}
                          value={endDate()}
                          onInput={(event) => setEndDate(event.currentTarget.value)}
                        />
                      </div>
                    </div>
                    <Show when={loaded().oldest_allowed_date}>
                      <p class={fieldHelp}>
                        Your plan keeps data back to {loaded().oldest_allowed_date}, so an import
                        can't start earlier.
                      </p>
                    </Show>
                    <Show when={!nothingToImport() && rangeProblem()}>
                      <p class={`${statusMessage} ${statusError}`}>{rangeProblem()}</p>
                    </Show>
                    <Show when={nothingToImport()}>
                      <p class={warning} role="status">
                        RevScope has tracked this site for your plan's whole retention period, so
                        there's nothing to import without duplicating visits and pageviews. Choose
                        an end date to import anyway.
                      </p>
                    </Show>
                    <Show when={duplicates()}>
                      <p class={warning} role="alert">
                        RevScope has been tracking this site since {loaded().first_tracked_date}.
                        Importing up to {endDate()} counts the days from then on twice, so visits
                        and pageviews on those days will be duplicated.
                      </p>
                    </Show>
                    <div class={field}>
                      <label class={fieldLabel} for="ga-hostnames">
                        Hostnames
                      </label>
                      <input
                        id="ga-hostnames"
                        class={input}
                        type="text"
                        placeholder="example.com, www.example.com"
                        value={hostnames()}
                        onInput={(event) => setHostnames(event.currentTarget.value)}
                      />
                      <p class={fieldHelp}>
                        Comma separated. Only traffic to these hostnames is imported. Leave empty to
                        import every hostname on the property.
                      </p>
                    </div>
                    <div>
                      <button
                        type="button"
                        class={saveButton}
                        disabled={busy() || !propertyId() || rangeProblem() !== ""}
                        onClick={() => void start()}
                      >
                        Start import
                      </button>
                    </div>
                  </div>
                </Show>
              )}
            </Show>

            <Show when={loaded().imports.length > 0}>
              <label class={toggleRow}>
                <input
                  type="checkbox"
                  checked={loaded().include_in_reports}
                  disabled={busy()}
                  onChange={(event) => void setIncluded(event.currentTarget.checked)}
                />
                Include imported data in reports
              </label>
              <ul class={importList}>
                <For each={loaded().imports}>
                  {(item) => (
                    <li class={importRow}>
                      <div>
                        <div class={importName}>{item.property_name}</div>
                        <p class={importDetail}>
                          {item.start_date} to {item.end_date} ·{" "}
                          {item.sessions_imported.toLocaleString()} sessions · {statusText(item)}
                        </p>
                        <Show when={item.status === "failed" && item.error}>
                          <p class={importError}>{item.error}</p>
                        </Show>
                      </div>
                      <div class={importActions}>
                        <Show
                          when={isActive(item)}
                          fallback={
                            <button
                              type="button"
                              class={inlineButton}
                              disabled={busy()}
                              onClick={() =>
                                confirmingDelete() === item.id
                                  ? void remove(item.id)
                                  : setConfirmingDelete(item.id)
                              }
                            >
                              {confirmingDelete() === item.id ? "Really delete?" : "Delete"}
                            </button>
                          }
                        >
                          <button
                            type="button"
                            class={inlineButton}
                            disabled={busy()}
                            onClick={() => void cancel(item.id)}
                          >
                            Cancel
                          </button>
                        </Show>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
