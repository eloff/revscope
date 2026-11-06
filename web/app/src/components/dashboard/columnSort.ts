export type SortDirection = "asc" | "desc";

export type ColumnSort<Column> = { column: Column; direction: SortDirection };

/**
 * The sort after a header click: the active column flips direction, and a new
 * column starts in `initialDirection`.
 */
export function nextColumnSort<Column>(
  current: ColumnSort<Column>,
  column: Column,
  initialDirection: SortDirection = "desc",
): ColumnSort<Column> {
  if (current.column === column) {
    return { column, direction: current.direction === "desc" ? "asc" : "desc" };
  }
  return { column, direction: initialDirection };
}
