import { fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type { CampaignPreviewResponse, CampaignSpendEntry } from "../../lib/campaignsApi";
import CampaignEditor, {
  type CampaignDraft,
  draftRules,
  emptyRule,
  rulesFromCampaign,
} from "./CampaignEditor";
import CampaignSpend from "./CampaignSpend";
import SpendEditor from "./SpendEditor";

describe("SpendEditor", () => {
  it("starts with one blank row and saves valid spend in thousandths", async () => {
    const onSave = vi.fn(async () => {});
    render(() => (
      <SpendEditor initialRows={[]} readOnly={false} onCancel={vi.fn()} onSave={onSave} />
    ));
    const user = userEvent.setup();
    fireEvent.input(screen.getByLabelText("Date"), { target: { value: "2026-02-01" } });
    await user.type(screen.getByLabelText("Amount"), "12.50");
    await user.click(screen.getByRole("button", { name: "Save spend" }));
    expect(onSave).toHaveBeenCalledWith([
      { kind: "once", start_date: "2026-02-01", amount: 12_500 },
    ]);
  });

  it("switches a row to a daily range", async () => {
    const onSave = vi.fn(async () => {});
    render(() => (
      <SpendEditor initialRows={[]} readOnly={false} onCancel={vi.fn()} onSave={onSave} />
    ));
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Spend type"), "daily");
    fireEvent.input(screen.getByLabelText("Start date"), { target: { value: "2026-02-01" } });
    fireEvent.input(screen.getByLabelText("End date"), { target: { value: "2026-02-10" } });
    await user.type(screen.getByLabelText("Amount per day"), "5");
    await user.click(screen.getByRole("button", { name: "Save spend" }));
    expect(onSave).toHaveBeenCalledWith([
      { kind: "daily", start_date: "2026-02-01", end_date: "2026-02-10", amount: 5000 },
    ]);
  });

  it("flags the invalid row and clears the error on edit", async () => {
    const onSave = vi.fn(async () => {});
    const { container } = render(() => (
      <SpendEditor initialRows={[]} readOnly={false} onCancel={vi.fn()} onSave={onSave} />
    ));
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Amount"), "5");
    await user.click(screen.getByRole("button", { name: "Save spend" }));
    expect(screen.getByText("Spend 1: choose a date.")).toBeInTheDocument();
    expect(container.querySelector('[data-invalid="true"]')).not.toBeNull();
    expect(onSave).not.toHaveBeenCalled();

    await user.type(screen.getByLabelText("Amount"), "0");
    expect(screen.queryByText("Spend 1: choose a date.")).toBeNull();
  });

  it("adds and removes rows, and shows a failed save", async () => {
    const onSave = vi.fn(async () => {
      throw new Error("Spend overlaps another campaign.");
    });
    render(() => (
      <SpendEditor initialRows={[]} readOnly={false} onCancel={vi.fn()} onSave={onSave} />
    ));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "+ Add spend" }));
    expect(screen.getAllByLabelText("Amount")).toHaveLength(2);
    await user.click(screen.getAllByRole("button", { name: "Remove spend" })[1] as HTMLElement);
    expect(screen.getAllByLabelText("Amount")).toHaveLength(1);

    fireEvent.input(screen.getByLabelText("Date"), { target: { value: "2026-02-01" } });
    await user.type(screen.getByLabelText("Amount"), "1");
    await user.click(screen.getByRole("button", { name: "Save spend" }));
    expect(await screen.findByText("Spend overlaps another campaign.")).toBeInTheDocument();
  });

  it("is read-only on the demo", () => {
    render(() => <SpendEditor initialRows={[]} readOnly onCancel={vi.fn()} onSave={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Save spend" })).toBeDisabled();
    expect(screen.getByText("The demo project is read-only.")).toBeInTheDocument();
  });
});

describe("CampaignSpend", () => {
  const entries: CampaignSpendEntry[] = [
    {
      id: 1,
      kind: "once",
      start_date: "2026-01-05",
      end_date: "2026-01-05",
      amount: 25_000,
      spent_to_date: 25_000,
    },
    {
      id: 2,
      kind: "daily",
      start_date: "2026-02-01",
      end_date: "2026-02-28",
      amount: 3_000,
      spent_to_date: 30_000,
    },
  ];

  it("prompts for spend when there is none", () => {
    render(() => <CampaignSpend entries={[]} canEdit onSave={vi.fn()} />);
    expect(screen.getByText(/No spend entered/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add spend" })).toBeInTheDocument();
  });

  it("lists entries with spend to date", () => {
    render(() => <CampaignSpend entries={entries} canEdit onSave={vi.fn()} />);
    expect(screen.getByText(/^\$25 on /)).toBeInTheDocument();
    expect(screen.getByText(/^\$3\/day, /)).toBeInTheDocument();
    expect(screen.getByText("$30 to date")).toBeInTheDocument();
  });

  it("edits, saves and returns to the list", async () => {
    const onSave = vi.fn(async () => {});
    render(() => <CampaignSpend entries={entries} canEdit onSave={onSave} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Edit spend" }));
    expect(screen.getAllByLabelText(/Amount/)).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "Save spend" }));
    expect(onSave).toHaveBeenCalledWith([
      { kind: "once", start_date: "2026-01-05", amount: 25_000 },
      { kind: "daily", start_date: "2026-02-01", end_date: "2026-02-28", amount: 3_000 },
    ]);
    expect(await screen.findByRole("button", { name: "Edit spend" })).toBeInTheDocument();
  });

  it("cancels editing", async () => {
    render(() => <CampaignSpend entries={entries} canEdit onSave={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit spend" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("$30 to date")).toBeInTheDocument();
  });
});

describe("campaign rule drafts", () => {
  it("round-trips rules and drops blank values", () => {
    expect(rulesFromCampaign([])).toEqual([emptyRule()]);
    expect(rulesFromCampaign([{ param_key: "ref", param_value: "hn" }])).toEqual([
      { paramKey: "ref", paramValue: "hn" },
    ]);
    expect(
      draftRules([
        { paramKey: "ref", paramValue: " hn " },
        { paramKey: "source", paramValue: "  " },
      ]),
    ).toEqual([{ param_key: "ref", param_value: "hn" }]);
  });
});

describe("CampaignEditor", () => {
  const preview: CampaignPreviewResponse = {
    unclassified: 40,
    direct: 30,
    referrer: 10,
    already_classified: 5,
  };

  function renderEditor(initial: CampaignDraft, previewCampaign = vi.fn(async () => preview)) {
    const [draft, setDraft] = createSignal(initial);
    const onSave = vi.fn();
    render(() => (
      <CampaignEditor
        draft={draft()}
        saving={false}
        readOnly={false}
        error=""
        onChange={setDraft}
        onCancel={vi.fn()}
        onSave={onSave}
        previewCampaign={previewCampaign}
      />
    ));
    return { draft, onSave, previewCampaign };
  }

  it("previews how many visits the rules would claim", async () => {
    const { previewCampaign } = renderEditor({ name: "", rules: [emptyRule()] });
    expect(screen.getByRole("heading")).toHaveTextContent("New campaign");
    await userEvent.type(screen.getByLabelText("Parameter value"), "betalist");
    expect(
      await screen.findByText(/visits in this period would join this campaign/),
    ).toHaveTextContent(
      "40 visits in this period would join this campaign (30 direct, 10 with a referrer). 5 more match but already belong to another campaign.",
    );
    expect(previewCampaign).toHaveBeenLastCalledWith([
      { param_key: "source", param_value: "betalist" },
    ]);
  });

  it("debounces the preview while typing", async () => {
    const { previewCampaign } = renderEditor({ name: "", rules: [emptyRule()] });
    await userEvent.type(screen.getByLabelText("Parameter value"), "abc");
    await screen.findByText(/would join this campaign/);
    expect(previewCampaign).toHaveBeenCalledTimes(1);
  });

  it("shows a preview failure", async () => {
    renderEditor(
      { name: "", rules: [{ paramKey: "ref", paramValue: "hn" }] },
      vi.fn(async () => {
        throw new Error("Preview unavailable.");
      }),
    );
    expect(await screen.findByText("Preview unavailable.")).toBeInTheDocument();
  });

  it("adds OR conditions and keeps at least one", async () => {
    const { draft } = renderEditor({ id: 3, name: "Launch", rules: [emptyRule()] });
    expect(screen.getByRole("heading")).toHaveTextContent("Edit campaign");
    expect(screen.getByRole("button", { name: "Remove condition" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "+ Add OR condition" }));
    expect(screen.getByText("OR")).toBeInTheDocument();
    expect(screen.getByText("Match visits that landed with any of these")).toBeInTheDocument();
    await userEvent.selectOptions(
      screen.getAllByLabelText("Query parameter")[1] as HTMLElement,
      "ref",
    );
    expect(draft().rules[1]).toEqual({ paramKey: "ref", paramValue: "" });
    await userEvent.click(
      screen.getAllByRole("button", { name: "Remove condition" })[0] as HTMLElement,
    );
    expect(draft().rules).toEqual([{ paramKey: "ref", paramValue: "" }]);
  });

  it("only renames an auto-detected campaign", async () => {
    const { onSave, previewCampaign } = renderEditor({
      id: 3,
      name: "spring",
      rules: [{ paramKey: "source", paramValue: "x" }],
      renameOnly: true,
    });
    expect(screen.getByRole("heading")).toHaveTextContent("Rename campaign");
    expect(screen.queryByLabelText("Parameter value")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Save campaign" }));
    expect(onSave).toHaveBeenCalledOnce();
    expect(previewCampaign).not.toHaveBeenCalled();
  });
});
