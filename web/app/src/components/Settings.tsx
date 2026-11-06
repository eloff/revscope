/// <reference types="solid-js" />
import { Dialog } from "@ark-ui/solid/dialog";
import { createListCollection, Select } from "@ark-ui/solid/select";
import { Tabs } from "@ark-ui/solid/tabs";
import { patchProjectTimezone, useAuthState } from "@revscope/web-shared/authState";
import { appPath } from "@revscope/web-shared/links";
import {
  backdrop,
  content,
  description,
  title as dialogTitle,
  positioner,
} from "@revscope/web-shared/loginDialogStyles";
import {
  type ProjectOption,
  projectOptions,
  resolveProjectId,
  selectedProjectId,
  setSelectedProjectId,
} from "@revscope/web-shared/projectSelection";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { Portal } from "solid-js/web";
import {
  fetchSettings,
  INTEGRATION_META,
  INTEGRATION_TYPES,
  type IntegrationField,
  type IntegrationSavePayload,
  type IntegrationType,
  type SaveSettingsPayload,
  type SettingsResponse,
  saveSettings,
  timezoneOptions,
} from "../lib/settingsApi";
import GoogleAnalyticsImport from "./GoogleAnalyticsImport";
import { content as selectContent, item as selectItem } from "./ProjectSelect.css";
import {
  actionsBar,
  connectionStatus,
  connectionStatusConnected,
  dialogActions,
  dialogDanger,
  dialogPrimary,
  dialogSecondary,
  field,
  fieldGrid,
  fieldHelp,
  fieldLabel,
  header,
  heading,
  inlineButton,
  input,
  integrationStatus,
  maskedValue,
  message,
  optionalTag,
  page,
  saveButton,
  secretRow,
  section,
  sectionDescription,
  sectionTitle,
  selectIndicator,
  selectPositioner,
  selectTrigger,
  statusError,
  statusMessage,
  statusSuccess,
  subsectionTitle,
  subtitle,
  syncStatus,
  syncStatusError,
  tabConnected,
  tabList,
  tabsRoot,
  tabTrigger,
  timezoneField,
} from "./Settings.css";
import SettingsDangerZone from "./SettingsDangerZone";

type SecretFieldState = {
  value: string;
  dirty: boolean;
  masked: string | null;
};

type IntegrationFormState = {
  external_account_id: string;
  api_key: SecretFieldState;
  secret: SecretFieldState;
  last_synced_at: string | null;
  last_sync_error: string | null;
};

type FormState = {
  timezone: string;
  payment_success_path: string;
  custom_domain: string;
  integrations: Record<IntegrationType, IntegrationFormState>;
};

const EMPTY_RESPONSE: SettingsResponse = {
  timezone: "UTC",
  payment_success_path: null,
  custom_domain: null,
  integrations: [],
};

/** A navigation or project switch held back by the unsaved-changes dialog. */
type PendingAction = { kind: "navigate"; url: string } | { kind: "project"; projectId: string };

function buildForm(response: SettingsResponse): FormState {
  const integrations = {} as Record<IntegrationType, IntegrationFormState>;
  for (const type of INTEGRATION_TYPES) {
    const existing = response.integrations.find((entry) => entry.integration_type === type);
    integrations[type] = {
      external_account_id: existing?.external_account_id ?? "",
      api_key: {
        value: "",
        dirty: false,
        masked: existing?.api_key_masked ?? null,
      },
      secret: {
        value: "",
        dirty: false,
        masked: existing?.secret_masked ?? null,
      },
      last_synced_at: existing?.last_synced_at ?? null,
      last_sync_error: existing?.last_sync_error ?? null,
    };
  }
  return {
    timezone: response.timezone || "UTC",
    payment_success_path: response.payment_success_path ?? "",
    custom_domain: response.custom_domain ?? "",
    integrations,
  };
}

function emptyToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

function secretPayload(state: SecretFieldState): string | null {
  if (!state.dirty) {
    return null;
  }
  return state.value;
}

function integrationConnected(state: IntegrationFormState): boolean {
  return Boolean(state.api_key.masked || state.secret.masked);
}

/** Formats an API timestamp for relative display; falls back to absolute local time. */
function formatSyncTime(value: string): { relative: string; absolute: string } {
  const date = new Date(value);
  const absolute = Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      });
  if (Number.isNaN(date.getTime())) {
    return { relative: value, absolute };
  }
  const deltaSeconds = Math.round((date.getTime() - Date.now()) / 1000);
  const absSeconds = Math.abs(deltaSeconds);
  const divisions: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 60 * 60 * 24 * 365],
    ["month", 60 * 60 * 24 * 30],
    ["week", 60 * 60 * 24 * 7],
    ["day", 60 * 60 * 24],
    ["hour", 60 * 60],
    ["minute", 60],
    ["second", 1],
  ];
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  for (const [unit, secondsInUnit] of divisions) {
    if (absSeconds >= secondsInUnit || unit === "second") {
      return {
        relative: formatter.format(Math.round(deltaSeconds / secondsInUnit), unit),
        absolute,
      };
    }
  }
  return { relative: absolute, absolute };
}

export default function Settings() {
  const { authUser, authResolved } = useAuthState();
  const [form, setForm] = createStore<FormState>(buildForm(EMPTY_RESPONSE));
  const [activeTab, setActiveTab] = createSignal<IntegrationType>(INTEGRATION_TYPES[0]);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [statusErrorMessage, setStatusErrorMessage] = createSignal("");
  const [statusSuccessMessage, setStatusSuccessMessage] = createSignal("");
  const [dirty, setDirty] = createSignal(false);
  const [pendingAction, setPendingAction] = createSignal<PendingAction | null>(null);
  /** Follows the cached account, so a background `/v2/session/me` refresh that
   * adds or renames a project updates the header without reloading the form. */
  const projects = createMemo<ProjectOption[]>(() => {
    const user = authUser();
    return user ? projectOptions(user.projects) : [];
  });
  /** Settings currently loaded in the form; may lag `selectedProjectId` while the
   * unsaved-changes dialog is open after a switch from the account menu. */
  const [projectId, setProjectId] = createSignal("");
  const projectName = () => projects().find((option) => option.value === projectId())?.label ?? "";
  const timezoneCollection = createMemo(() =>
    createListCollection({ items: timezoneOptions(form.timezone) }),
  );

  let bypassGuard = false;

  const markDirty = () => {
    // Members can't save, so there is nothing for the unsaved-changes guard to keep.
    if (!authUser()?.is_admin) {
      return;
    }
    setDirty(true);
    setStatusSuccessMessage("");
  };

  const load = async (requestedProjectId?: string) => {
    const user = authUser();
    if (!user) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const options = projects();
      const preferred = requestedProjectId || projectId() || selectedProjectId();
      const target = resolveProjectId(options, preferred);
      if (!target) {
        setProjectId("");
        setLoadError("This account has no projects yet.");
        return;
      }
      setProjectId(target);
      setSelectedProjectId(target);
      const response = await fetchSettings(user, target);
      setForm(reconcile(buildForm(response)));
      setDirty(false);
      setLoadError("");
    } catch (caught) {
      setLoadError(caught instanceof Error ? caught.message : "Unable to load settings.");
    } finally {
      setLoading(false);
    }
  };

  const buildPayload = (): SaveSettingsPayload => {
    const integrations: IntegrationSavePayload[] = INTEGRATION_TYPES.map((type) => {
      const state = form.integrations[type];
      return {
        integration_type: type,
        external_account_id: emptyToNull(state.external_account_id),
        api_key: secretPayload(state.api_key),
        secret: secretPayload(state.secret),
      };
    });
    return {
      timezone: form.timezone,
      payment_success_path: emptyToNull(form.payment_success_path),
      custom_domain: emptyToNull(form.custom_domain),
      integrations,
    };
  };

  const handleSave = async (): Promise<boolean> => {
    const user = authUser();
    const project = projectId();
    if (!user || !project) {
      return false;
    }
    setSaving(true);
    setStatusErrorMessage("");
    setStatusSuccessMessage("");
    try {
      const response = await saveSettings(user, project, buildPayload());
      setForm(reconcile(buildForm(response)));
      patchProjectTimezone(project, response.timezone || "UTC");
      setDirty(false);
      setStatusSuccessMessage("Settings saved.");
      return true;
    } catch (caught) {
      setStatusErrorMessage(caught instanceof Error ? caught.message : "Unable to save settings.");
      return false;
    } finally {
      setSaving(false);
    }
  };

  /** Moves the form, and the selection the dashboard shares, onto a project that still exists. */
  const handleProjectDeleted = (deletedProjectId: string) => {
    setDirty(false);
    setStatusSuccessMessage("");
    setStatusErrorMessage("");
    const next = resolveProjectId(
      projects().filter((option) => option.value !== deletedProjectId),
      "",
    );
    setProjectId("");
    setSelectedProjectId(next);
    void load(next);
  };

  const applyPending = (action: PendingAction) => {
    setPendingAction(null);
    setDirty(false);
    if (action.kind === "navigate") {
      bypassGuard = true;
      window.location.href = action.url;
      return;
    }
    void load(action.projectId);
  };

  const dismissPending = () => {
    const action = pendingAction();
    setPendingAction(null);
    // Account menu already flipped `selectedProjectId`; put it back so the chip
    // matches the form still on screen.
    if (action?.kind === "project" && projectId()) {
      setSelectedProjectId(projectId());
    }
  };

  const onSaveAndContinue = async () => {
    const action = pendingAction();
    const saved = await handleSave();
    if (saved && action) {
      applyPending(action);
    } else {
      dismissPending();
    }
  };

  const onDiscardAndContinue = () => {
    const action = pendingAction();
    if (action) {
      applyPending(action);
    }
  };

  const onBeforeUnload = (event: BeforeUnloadEvent) => {
    if (!dirty() || bypassGuard) {
      return;
    }
    event.preventDefault();
    event.returnValue = "";
  };

  const onDocumentClick = (event: MouseEvent) => {
    if (!dirty() || bypassGuard || event.defaultPrevented) {
      return;
    }
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    const target = event.target as Element | null;
    const anchor = target?.closest("a");
    if (!anchor) {
      return;
    }
    const href = anchor.getAttribute("href");
    if (!href || href.startsWith("#") || href.startsWith("javascript:")) {
      return;
    }
    if (anchor.target && anchor.target !== "_self") {
      return;
    }
    if (anchor.hasAttribute("download")) {
      return;
    }
    const destination = new URL(anchor.href, window.location.href);
    const samePage =
      destination.pathname === window.location.pathname &&
      destination.search === window.location.search;
    if (samePage) {
      return;
    }
    event.preventDefault();
    setPendingAction({ kind: "navigate", url: destination.href });
  };

  onMount(() => {
    document.addEventListener("click", onDocumentClick, true);
    window.addEventListener("beforeunload", onBeforeUnload);
  });

  /**
   * Keyed on the user rather than the session object so a background refresh of
   * the signed-in account does not reload the form out from under an edit.
   * Project switches come from the account menu via `selectedProjectId`.
   */
  const userId = createMemo(() => authUser()?.user_id ?? "");
  // The dashboard timezone link lands on #timezone, but this page is client-only,
  // so the target is not in the document when the browser first applies the hash.
  createEffect(() => {
    if (loading()) {
      return;
    }
    if (window.location.hash !== "#timezone") {
      return;
    }
    document.getElementById("timezone")?.scrollIntoView({ block: "nearest" });
  });

  createEffect(
    on([authResolved, userId, selectedProjectId], ([resolved, uid, selected]) => {
      if (!resolved || !uid) {
        return;
      }
      const target = resolveProjectId(projects(), selected);
      if (!target) {
        setProjectId("");
        setLoading(false);
        setLoadError("This account has no projects yet.");
        return;
      }
      if (target === projectId()) {
        return;
      }
      if (dirty() && projectId()) {
        setPendingAction({ kind: "project", projectId: target });
        return;
      }
      void load(target);
    }),
  );

  onCleanup(() => {
    if (typeof document !== "undefined") {
      document.removeEventListener("click", onDocumentClick, true);
    }
    if (typeof window !== "undefined") {
      window.removeEventListener("beforeunload", onBeforeUnload);
    }
  });

  const renderTextField = (
    type: IntegrationType,
    column: "external_account_id",
    meta: IntegrationField,
    fieldId: string,
  ) => (
    <input
      id={fieldId}
      class={input}
      type="text"
      placeholder={meta.placeholder}
      value={form.integrations[type][column]}
      onInput={(event) => {
        setForm("integrations", type, column, event.currentTarget.value);
        markDirty();
      }}
    />
  );

  const renderSecretField = (
    type: IntegrationType,
    column: "api_key" | "secret",
    meta: IntegrationField,
    fieldId: string,
  ) => {
    const state = () => form.integrations[type][column];
    return (
      <Show
        when={!state().masked || state().dirty}
        fallback={
          <div class={secretRow}>
            <span class={maskedValue}>{state().masked}</span>
            <button
              id={fieldId}
              type="button"
              class={inlineButton}
              onClick={() => {
                setForm("integrations", type, column, {
                  dirty: true,
                  value: "",
                });
                markDirty();
              }}
            >
              Replace
            </button>
          </div>
        }
      >
        <div class={secretRow}>
          <input
            id={fieldId}
            class={input}
            style={{ flex: "1 1 auto" }}
            type="text"
            autocomplete="off"
            spellcheck={false}
            placeholder={meta.placeholder}
            value={state().value}
            onInput={(event) => {
              setForm("integrations", type, column, {
                dirty: true,
                value: event.currentTarget.value,
              });
              markDirty();
            }}
          />
          <Show when={state().masked}>
            <button
              type="button"
              class={inlineButton}
              onClick={() => {
                setForm("integrations", type, column, {
                  dirty: false,
                  value: "",
                });
              }}
            >
              Cancel
            </button>
          </Show>
        </div>
      </Show>
    );
  };

  const renderSyncDetail = (type: IntegrationType) => {
    const state = form.integrations[type];
    if (state.last_sync_error) {
      const lastSyncedAt = state.last_synced_at;
      const succeeded = lastSyncedAt ? formatSyncTime(lastSyncedAt) : null;
      return (
        <p class={syncStatusError} role="status">
          Last sync failed: {state.last_sync_error}
          <Show when={succeeded && lastSyncedAt}>
            {" "}
            (last succeeded{" "}
            <time datetime={lastSyncedAt ?? undefined} title={succeeded?.absolute}>
              {succeeded?.relative}
            </time>
            )
          </Show>
        </p>
      );
    }
    if (state.last_synced_at) {
      const synced = formatSyncTime(state.last_synced_at);
      return (
        <p class={syncStatus} role="status">
          Last synced{" "}
          <time datetime={state.last_synced_at} title={synced.absolute}>
            {synced.relative}
          </time>
        </p>
      );
    }
    return (
      <p class={syncStatus} role="status">
        Waiting for first sync…
      </p>
    );
  };

  const renderIntegrationStatus = (type: IntegrationType) => {
    const connected = integrationConnected(form.integrations[type]);
    return (
      <div class={integrationStatus}>
        <p
          class={connected ? `${connectionStatus} ${connectionStatusConnected}` : connectionStatus}
          role="status"
        >
          {connected ? "Connected" : "Not connected"}
        </p>
        <Show when={connected}>{renderSyncDetail(type)}</Show>
      </div>
    );
  };

  const renderField = (type: IntegrationType, fieldMeta: IntegrationField) => {
    const fieldId = `settings-${type}-${fieldMeta.column}`;
    return (
      <div class={field}>
        <label class={fieldLabel} for={fieldId}>
          {fieldMeta.label}
        </label>
        {fieldMeta.secret
          ? renderSecretField(type, fieldMeta.column as "api_key" | "secret", fieldMeta, fieldId)
          : renderTextField(type, fieldMeta.column as "external_account_id", fieldMeta, fieldId)}
        <Show when={fieldMeta.help}>
          <p class={fieldHelp}>{fieldMeta.help}</p>
        </Show>
      </div>
    );
  };

  return (
    <div class={page}>
      <header class={header}>
        <h1 class={heading}>Settings</h1>
        <p class={subtitle}>Manage integrations and your account preferences.</p>
      </header>

      <Show when={authResolved()} fallback={<p class={message}>Loading settings…</p>}>
        <Show when={authUser()} fallback={<p class={message}>Log in to manage your settings.</p>}>
          <Show when={!loading()} fallback={<p class={message}>Loading settings…</p>}>
            <Show when={loadError()}>
              <p class={`${statusMessage} ${statusError}`}>{loadError()}</p>
            </Show>

            <div>
              <section class={section}>
                <h2 class={sectionTitle}>
                  {projectName() ? `${projectName()} settings` : "Project settings"}
                </h2>
                <p class={sectionDescription}>
                  Reporting timezone, checkout tracking, first-party domain, and connected data
                  sources.
                </p>
                <div class={fieldGrid}>
                  <div class={timezoneField} id="timezone">
                    <span class={fieldLabel} id="settings-timezone-label">
                      Timezone
                    </span>
                    <Select.Root
                      collection={timezoneCollection()}
                      value={[form.timezone]}
                      onValueChange={(details) => {
                        const next = details.value[0];
                        if (next) {
                          setForm("timezone", next);
                          markDirty();
                        }
                      }}
                    >
                      <Select.Control>
                        <Select.Trigger
                          class={selectTrigger}
                          aria-labelledby="settings-timezone-label"
                        >
                          <Select.ValueText placeholder="Select timezone" />
                          <Select.Indicator class={selectIndicator} aria-hidden="true">
                            ▾
                          </Select.Indicator>
                        </Select.Trigger>
                      </Select.Control>
                      <Portal>
                        <Select.Positioner class={selectPositioner}>
                          <Select.Content class={selectContent}>
                            <For each={timezoneCollection().items}>
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
                    <p class={fieldHelp}>
                      Controls how dates and times are shown for this project across the dashboard.
                    </p>
                  </div>
                  <div class={field}>
                    <label class={fieldLabel} for="settings-payment-success-path">
                      Payment success path
                      <span class={optionalTag}>optional</span>
                    </label>
                    <input
                      id="settings-payment-success-path"
                      class={input}
                      type="text"
                      placeholder="/checkout/success"
                      value={form.payment_success_path}
                      onInput={(event) => {
                        setForm("payment_success_path", event.currentTarget.value);
                        markDirty();
                      }}
                    />
                    <p class={fieldHelp}>
                      Used to match unattributed payments to a session that reached your checkout
                      success page. Set it to the path your checkout flow redirects to after a
                      successful payment.
                    </p>
                  </div>
                  <div class={field}>
                    <label class={fieldLabel} for="settings-custom-domain">
                      Custom domain
                      <span class={optionalTag}>optional</span>
                    </label>
                    <input
                      id="settings-custom-domain"
                      class={input}
                      type="text"
                      inputMode="url"
                      spellcheck={false}
                      autocomplete="off"
                      placeholder="assets.example.com"
                      value={form.custom_domain}
                      onInput={(event) => {
                        setForm("custom_domain", event.currentTarget.value);
                        markDirty();
                      }}
                    />
                    <p class={fieldHelp}>
                      First-party hostname for the tracker. Create a DNS-only CNAME from this name
                      to <code>eu1.revscope.co</code>, then use it as the server URL in the install
                      snippet. Leave empty to use the hosted <code>eu1.revscope.co</code> endpoint.
                    </p>
                  </div>
                </div>

                <h3 class={subsectionTitle}>Integrations</h3>
                <p class={sectionDescription}>
                  Connect revenue and search data sources. Keys and secrets are encrypted and never
                  shown again after saving. Please allow several minutes for synced data to appear.
                </p>
                <Tabs.Root
                  value={activeTab()}
                  onValueChange={(details) => setActiveTab(details.value as IntegrationType)}
                  class={tabsRoot}
                >
                  <Tabs.List class={tabList}>
                    <For each={INTEGRATION_TYPES}>
                      {(type) => (
                        <Tabs.Trigger value={type} class={tabTrigger}>
                          {INTEGRATION_META[type].label}
                          <Show when={integrationConnected(form.integrations[type])}>
                            <span class={tabConnected}>Connected</span>
                          </Show>
                        </Tabs.Trigger>
                      )}
                    </For>
                  </Tabs.List>
                  <For each={INTEGRATION_TYPES}>
                    {(type) => (
                      <Tabs.Content value={type}>
                        <p class={sectionDescription}>{INTEGRATION_META[type].description}</p>
                        {renderIntegrationStatus(type)}
                        <div class={fieldGrid}>
                          <For each={INTEGRATION_META[type].fields}>
                            {(fieldMeta) => renderField(type, fieldMeta)}
                          </For>
                        </div>
                      </Tabs.Content>
                    )}
                  </For>
                </Tabs.Root>
              </section>

              <Show
                when={authUser()?.is_admin}
                fallback={<p class={message}>Only account admins can change these settings.</p>}
              >
                <div class={actionsBar}>
                  <button
                    type="button"
                    class={saveButton}
                    disabled={saving() || !dirty()}
                    onClick={() => void handleSave()}
                  >
                    {saving() ? "Saving…" : "Save"}
                  </button>
                  <Show when={statusErrorMessage()}>
                    <p class={`${statusMessage} ${statusError}`}>{statusErrorMessage()}</p>
                  </Show>
                  <Show when={statusSuccessMessage()}>
                    <p class={`${statusMessage} ${statusSuccess}`}>{statusSuccessMessage()}</p>
                  </Show>
                </div>
              </Show>
            </div>

            <Show when={authUser()?.is_admin}>
              <section class={section}>
                <h2 class={sectionTitle}>Users</h2>
                <p class={sectionDescription}>
                  Invite people to this account and choose which projects they can see.
                </p>
                <a href={appPath("/settings/users")}>Manage users</a>
              </section>
            </Show>
            <Show when={authUser()?.is_admin ? authUser() : undefined}>
              {(user) => (
                <Show when={projectId()} keyed>
                  {(id) => <GoogleAnalyticsImport session={user()} projectId={id} />}
                </Show>
              )}
            </Show>
            <Show when={authUser()?.is_admin && authUser()}>
              {(user) => (
                <SettingsDangerZone
                  session={user()}
                  project={
                    projectId() && projectName()
                      ? { id: projectId(), name: projectName() }
                      : undefined
                  }
                  projectCount={projects().length}
                  onProjectDeleted={handleProjectDeleted}
                />
              )}
            </Show>
          </Show>
        </Show>
      </Show>

      <Dialog.Root
        open={pendingAction() !== null}
        onOpenChange={(details) => {
          if (!details.open) {
            dismissPending();
          }
        }}
      >
        <Dialog.Backdrop class={backdrop} />
        <Dialog.Positioner class={positioner}>
          <Dialog.Content class={content}>
            <Dialog.Title class={dialogTitle}>Unsaved changes</Dialog.Title>
            <Dialog.Description class={description}>
              You have unsaved changes. Would you like to save them first?
            </Dialog.Description>
            <div class={dialogActions}>
              <button
                type="button"
                class={dialogPrimary}
                disabled={saving()}
                onClick={() => void onSaveAndContinue()}
              >
                {saving() ? "Saving…" : "Save changes"}
              </button>
              <button type="button" class={dialogDanger} onClick={onDiscardAndContinue}>
                Discard changes
              </button>
              <button type="button" class={dialogSecondary} onClick={dismissPending}>
                Cancel
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Positioner>
      </Dialog.Root>
    </div>
  );
}
