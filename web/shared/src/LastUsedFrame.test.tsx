import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import LastUsedFrame from "./LastUsedFrame";
import { frame } from "./LastUsedFrame.css";

describe("LastUsedFrame", () => {
  it("draws the frame and legend around its children when active", () => {
    render(() => (
      <LastUsedFrame active={true}>
        <button type="button">Sign in</button>
      </LastUsedFrame>
    ));
    const wrapper = screen.getByRole("button", { name: "Sign in" }).parentElement;
    expect(wrapper).toHaveClass(frame);
    expect(wrapper).toContainElement(screen.getByText("Last used"));
  });

  it("draws neither when inactive, and still renders its children", () => {
    render(() => (
      <LastUsedFrame active={false}>
        <button type="button">Sign in</button>
      </LastUsedFrame>
    ));
    expect(screen.getByRole("button", { name: "Sign in" }).parentElement).not.toHaveClass(frame);
    expect(screen.queryByText("Last used")).toBeNull();
  });
});
