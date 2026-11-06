import { style } from "@vanilla-extract/css";

export const userList = style({
  listStyle: "none",
  margin: "0 0 8px",
  padding: 0,
  border: "1px solid #e5e7eb",
  borderRadius: "8px",
});

export const userRow = style({
  padding: "12px 14px",
  selectors: {
    "& + &": {
      borderTop: "1px solid #e5e7eb",
    },
  },
});

export const userSummary = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "6px 12px",
});

export const userIdentity = style({
  display: "flex",
  flexDirection: "column",
  minWidth: 0,
  flex: "1 1 220px",
});

export const userName = style({
  fontWeight: 600,
});

export const userEmail = style({
  color: "#6b7280",
  fontSize: "0.9rem",
  overflow: "hidden",
  textOverflow: "ellipsis",
});

export const userAccessSummary = style({
  color: "#374151",
  fontSize: "0.85rem",
  flex: "1 1 180px",
});

export const userActions = style({
  display: "flex",
  gap: "8px",
  marginLeft: "auto",
});

const tag = style({
  padding: "2px 8px",
  borderRadius: "999px",
  fontSize: "0.75rem",
  fontWeight: 600,
  whiteSpace: "nowrap",
});

export const pendingTag = style([tag, { background: "#fef3c7", color: "#92400e" }]);

export const adminTag = style([tag, { background: "#dbeafe", color: "#1e40af" }]);

export const youTag = style([tag, { background: "#f3f4f6", color: "#374151" }]);

export const rowButton = style({
  padding: "6px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "6px",
  background: "white",
  color: "#111827",
  fontSize: "0.85rem",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
    "&:disabled": {
      opacity: 0.6,
      cursor: "not-allowed",
    },
  },
});

export const removeButton = style([rowButton, { color: "#b91c1c", borderColor: "#fecaca" }]);

export const editPanel = style({
  marginTop: "12px",
  paddingTop: "12px",
  borderTop: "1px dashed #e5e7eb",
});

export const editActions = style({
  display: "flex",
  alignItems: "center",
  gap: "12px",
  marginTop: "12px",
});

export const accessFieldset = style({
  border: 0,
  margin: 0,
  padding: 0,
  display: "grid",
  gap: "6px",
});

export const accessLegend = style({
  padding: 0,
  marginBottom: "4px",
  fontSize: "0.9rem",
  fontWeight: 600,
});

export const choice = style({
  display: "flex",
  alignItems: "baseline",
  gap: "8px",
  fontSize: "0.9rem",
  cursor: "pointer",
});

export const choiceHint = style({
  color: "#6b7280",
});

export const projectChoices = style({
  display: "grid",
  gap: "4px",
  margin: "4px 0 0 24px",
});

export const accessGroups = style({
  display: "grid",
  gap: "14px",
});

export const inviteLinkRow = style({
  display: "flex",
  gap: "8px",
  maxWidth: "720px",
  marginTop: "8px",
});

export const inviteLink = style({
  flex: 1,
  minWidth: 0,
});
