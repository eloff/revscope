import { createSignal } from "solid-js";

export type CursorHover<K = string> = ReturnType<typeof createCursorHover<K>>;

/** Which item a hover card describes, and the cursor point it is anchored to. */
export function createCursorHover<K = string>() {
  const [key, setKey] = createSignal<K | null>(null);
  const [x, setX] = createSignal(0);
  const [y, setY] = createSignal(0);
  return {
    key,
    x,
    y,
    show: (hoveredKey: K | null, clientX: number, clientY: number) => {
      setX(clientX);
      setY(clientY);
      // A function form keeps Solid from calling a function-valued key as an updater.
      setKey(() => hoveredKey);
    },
    /** Keeps an open card following the cursor. */
    move: (clientX: number, clientY: number) => {
      if (key() !== null) {
        setX(clientX);
        setY(clientY);
      }
    },
    clear: () => setKey(null),
  };
}
