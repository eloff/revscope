/// <reference types="solid-js" />
import { Tabs } from "@ark-ui/solid/tabs";
import type { AuthSession } from "@revscope/web-shared/authState";
import { docsUrl } from "@revscope/web-shared/links";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import {
  deleteFunnel,
  type Funnel,
  type FunnelDataResponse,
  type FunnelOptionsResponse,
  type FunnelPreviewRequest,
  fetchFunnelData,
  fetchFunnelDataBootstrap,
  fetchFunnelDataPreview,
  fetchFunnelOptions,
  type SaveFunnelRequest,
  saveFunnel,
} from "../../lib/funnelsApi";
import { hasSegmentFilters, segmentFiltersKey } from "../../lib/segmentFilters";
import { dangerButton, secondaryButton } from "../buttons.css";
import { useDashboardState } from "./dashboardPeriodContext";
import FunnelChart from "./FunnelChart";
import FunnelEditor, { type FunnelDraft } from "./FunnelEditor";
import {
  funnelActions,
  funnelAddTabButton,
  funnelDescription,
  funnelDocsLink,
  funnelEmptyState,
  funnelHeader,
  funnelPanel,
  funnelsShell,
  funnelTabBar,
  funnelTabButton,
  funnelTabRail,
} from "./Funnels.css";
import { createFunnelDataCache } from "./funnelDataCache";
import { errorMessage, message } from "./messages.css";
import TabDocsLink from "./TabDocsLink";

type FunnelsProps = {
  projectId: string;
  publicAccess?: boolean;
  session?: AuthSession;
};

/** One open editor; a fresh object per open so reopening resets the form. */
type EditSession = { funnel?: Funnel };

export default function Funnels(props: FunnelsProps) {
  const { range, rangeReady, previousRange, segments } = useDashboardState();
  const [funnels, setFunnels] = createSignal<Funnel[]>([]);
  const [selectedFunnelId, setSelectedFunnelId] = createSignal<number | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [loadError, setLoadError] = createSignal("");
  const [dataLoading, setDataLoading] = createSignal(false);
  const [dataError, setDataError] = createSignal("");
  const [selectedData, setSelectedData] = createSignal<FunnelDataResponse>();
  const [options, setOptions] = createSignal<FunnelOptionsResponse>();
  const [optionsLoading, setOptionsLoading] = createSignal(false);
  const [optionsError, setOptionsError] = createSignal("");
  const [editSession, setEditSession] = createSignal<EditSession>();
  const dataCache = createFunnelDataCache(() => {
    const current = range();
    const previous = previousRange();
    return `${current.start}:${current.end}:${previous.start}:${previous.end}:${segmentFiltersKey(segments())}`;
  });
  const selectedFunnel = createMemo(() => {
    const id = selectedFunnelId();
    return funnels().find((funnel) => funnel.id === id);
  });
  const selectedFunnelTabValue = createMemo(() => {
    const id = selectedFunnelId();
    return id === null ? "" : String(id);
  });

  const canCallAuthenticatedApi = () => props.publicAccess || props.session;
  // Demo (publicAccess) is always read-only — no create / edit / delete.
  const canEditFunnels = () => !props.publicAccess && Boolean(props.session);

  const requireSession = (): AuthSession => {
    const session = props.session;
    if (!session) {
      throw new Error("Not authenticated");
    }
    return session;
  };
  // The demo reads anonymously, since a signed-in viewer is not its owner.
  const readSession = (): AuthSession | undefined =>
    props.publicAccess ? undefined : requireSession();

  const [bootstrapped, setBootstrapped] = createSignal(false);
  let bootstrapToken = 0;

  /** One request: funnel list + data for the first funnel. */
  const loadBootstrap = async () => {
    if (!canCallAuthenticatedApi() || !rangeReady()) {
      return;
    }
    const token = ++bootstrapToken;
    const generation = dataCache.sync();
    setLoading(true);
    setDataLoading(true);
    setLoadError("");
    setDataError("");
    try {
      const current = range();
      const previous = previousRange();
      const response = await fetchFunnelDataBootstrap(
        readSession(),
        props.projectId,
        current.start,
        current.end,
        previous.start,
        previous.end,
        segments(),
      );
      if (token !== bootstrapToken) {
        return;
      }
      setFunnels(response.funnels);
      const nextId = response.data?.funnel_id ?? response.funnels[0]?.id ?? null;
      setSelectedFunnelId(nextId);
      if (response.data) {
        dataCache.write(generation, response.data);
        setSelectedData(response.data);
      } else {
        setSelectedData(undefined);
      }
      setBootstrapped(true);
    } catch (error) {
      if (token !== bootstrapToken) {
        return;
      }
      const errorText = error instanceof Error ? error.message : "Unable to load funnels.";
      setLoadError(errorText);
      setDataError(errorText);
      setSelectedData(undefined);
    } finally {
      if (token === bootstrapToken) {
        setLoading(false);
        setDataLoading(false);
      }
    }
  };

  const loadOptions = async () => {
    if (options() || optionsLoading() || !canCallAuthenticatedApi()) {
      return;
    }
    setOptionsLoading(true);
    setOptionsError("");
    try {
      const response = await fetchFunnelOptions(readSession(), props.projectId);
      setOptions(response);
    } catch (error) {
      setOptionsError(error instanceof Error ? error.message : "Unable to load funnel options.");
    } finally {
      setOptionsLoading(false);
    }
  };

  let selectedDataToken = 0;

  const loadSelectedData = async () => {
    // Switching funnels mid-request must not let the earlier answer win.
    const token = ++selectedDataToken;
    const generation = dataCache.sync();
    const funnel = selectedFunnel();
    if (!funnel || !canCallAuthenticatedApi()) {
      setSelectedData(undefined);
      setDataLoading(false);
      return;
    }
    const cached = dataCache.read(funnel.id);
    if (cached) {
      setSelectedData(cached);
      setDataError("");
      setDataLoading(false);
      return;
    }
    setDataLoading(true);
    setDataError("");
    try {
      const current = range();
      const previous = previousRange();
      const response = await fetchFunnelData(
        readSession(),
        props.projectId,
        funnel.id,
        current.start,
        current.end,
        previous.start,
        previous.end,
        segments(),
      );
      dataCache.write(generation, response);
      if (token === selectedDataToken) {
        setSelectedData(response);
      }
    } catch (error) {
      if (token === selectedDataToken) {
        setSelectedData(undefined);
        setDataError(error instanceof Error ? error.message : "Unable to load funnel data.");
      }
    } finally {
      if (token === selectedDataToken) {
        setDataLoading(false);
      }
    }
  };

  const fetchPreview = (request: FunnelPreviewRequest) =>
    fetchFunnelDataPreview(readSession(), props.projectId, request);

  // Initial load: funnel list + first funnel's data in one round-trip.
  createEffect(() => {
    range();
    previousRange();
    segments();
    if (!rangeReady() || !canCallAuthenticatedApi() || bootstrapped()) {
      return;
    }
    void loadBootstrap();
  });

  createEffect(() => {
    selectedFunnelId();
    range();
    previousRange();
    segments();
    if (!rangeReady() || !bootstrapped()) {
      return;
    }
    void loadSelectedData();
  });

  const openEditor = (funnel?: Funnel) => {
    if (!canEditFunnels()) {
      return;
    }
    setEditSession({ funnel });
    void loadOptions();
  };

  const saveDraft = async (draft: FunnelDraft) => {
    if (!canEditFunnels()) {
      return;
    }
    const current = range();
    const previous = previousRange();
    const request: SaveFunnelRequest = {
      id: editSession()?.funnel?.id,
      ...draft,
      start: current.start,
      end: current.end,
      previous_start: previous.start,
      previous_end: previous.end,
    };
    const generation = dataCache.sync();
    const saved = await saveFunnel(requireSession(), props.projectId, request);
    setFunnels((currentFunnels) => {
      const index = currentFunnels.findIndex((funnel) => funnel.id === saved.funnel.id);
      if (index === -1) {
        return [...currentFunnels, saved.funnel];
      }
      const next = [...currentFunnels];
      next[index] = saved.funnel;
      return next;
    });
    // Compared after the await: the range may have moved while saving.
    const sameRange = generation === dataCache.sync();
    // A data request still in flight describes the steps before this save,
    // so it must not cache over the saved funnel's data when it lands.
    const savedGeneration = dataCache.invalidate(saved.funnel.id);
    // The save answers for the whole project: it ignores segment filters.
    if (sameRange && !hasSegmentFilters(segments())) {
      dataCache.write(savedGeneration, saved.data);
    }
    setSelectedFunnelId(saved.funnel.id);
    setEditSession(undefined);
    // Supersedes that in-flight request on screen too: shows the saved data
    // from cache, or refetches it when the range moved or filters are on.
    void loadSelectedData();
  };

  const deleteSelected = async (funnel: Funnel) => {
    if (!canEditFunnels()) {
      return;
    }
    if (
      typeof window !== "undefined" &&
      !window.confirm(
        `Delete "${funnel.name}"? This cannot be undone and all funnel steps will be removed.`,
      )
    ) {
      return;
    }
    setLoadError("");
    try {
      const response = await deleteFunnel(requireSession(), props.projectId, funnel.id);
      setFunnels(response.funnels);
      dataCache.clear();
      setSelectedFunnelId(response.funnels[0]?.id ?? null);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : "Unable to delete funnel.");
    }
  };

  const selectFunnelTab = (value: string) => {
    if (!value) {
      return;
    }
    setSelectedFunnelId(Number(value));
    setEditSession(undefined);
  };

  return (
    <Tabs.Root
      value={selectedFunnelTabValue()}
      onValueChange={(details) => selectFunnelTab(details.value)}
      class={funnelsShell}
    >
      <div class={funnelTabBar}>
        <Tabs.List class={funnelTabRail} aria-label="Funnels">
          <For each={funnels()}>
            {(funnel) => (
              <Tabs.Trigger value={String(funnel.id)} class={funnelTabButton}>
                {funnel.name}
              </Tabs.Trigger>
            )}
          </For>
        </Tabs.List>
        <Show when={canEditFunnels()}>
          <button type="button" class={funnelAddTabButton} onClick={() => openEditor()}>
            + Create funnel
          </button>
        </Show>
        <TabDocsLink href={docsUrl("/docs/funnels")} label="Funnel Docs" class={funnelDocsLink} />
      </div>

      <Show when={loading()}>
        <p class={message}>Loading funnels...</p>
      </Show>

      <Tabs.Content value={selectedFunnelTabValue()} class={funnelPanel}>
        <Show when={loadError()}>{(error) => <p class={errorMessage}>{error()}</p>}</Show>

        <Show
          when={editSession()}
          keyed
          fallback={
            <Show
              when={selectedFunnel()}
              fallback={
                <div class={funnelEmptyState}>
                  <p>
                    {canEditFunnels()
                      ? "No funnels yet. Add a funnel to start tracking drop-off and conversion."
                      : "No funnels in this project yet."}
                  </p>
                </div>
              }
            >
              {(funnel) => (
                <>
                  <Show when={funnel().description || canEditFunnels()}>
                    <div class={funnelHeader}>
                      <div>
                        <Show when={funnel().description}>
                          {(description) => <p class={funnelDescription}>{description()}</p>}
                        </Show>
                      </div>
                      <Show when={canEditFunnels()}>
                        <div class={funnelActions}>
                          <button
                            type="button"
                            class={secondaryButton}
                            onClick={() => openEditor(funnel())}
                          >
                            Edit
                          </button>
                          <button
                            type="button"
                            class={dangerButton}
                            onClick={() => void deleteSelected(funnel())}
                          >
                            Delete
                          </button>
                        </div>
                      </Show>
                    </div>
                  </Show>
                  <FunnelChart data={selectedData()} loading={dataLoading()} error={dataError()} />
                </>
              )}
            </Show>
          }
        >
          {(session) => (
            <FunnelEditor
              funnel={session.funnel}
              options={options()}
              optionsLoading={optionsLoading()}
              optionsError={optionsError()}
              fetchPreview={fetchPreview}
              onCancel={() => setEditSession(undefined)}
              onSave={saveDraft}
            />
          )}
        </Show>
      </Tabs.Content>
    </Tabs.Root>
  );
}
