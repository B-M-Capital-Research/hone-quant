import type { JSX } from "solid-js";

/** Extra stroke icons for the activity pages, drawn in the same 24×24 / 1.7px language as `Icon`. */
const PATHS = {
  pencil: "M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z",
  octagon: "M7.86 2h8.28L22 7.86v8.28L16.14 22H7.86L2 16.14V7.86zM12 8v4M12 16h.01",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  check_all: "M17 6 6.5 16.5 2 12M22 8l-8.5 8.5-1.5-1.5",
  terminal: "M4 17l6-6-6-6M12 19h8",
  cpu: "M9 3v2M15 3v2M9 19v2M15 19v2M3 9h2M3 15h2M19 9h2M19 15h2M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zM10 10h4v4h-4z",
  copy: "M9 9h11v11H9zM5 15H4V4h11v1",
  arrow_right: "M5 12h14M13 6l6 6-6 6",
  moon_clock: "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z",
  repeat: "M17 2l4 4-4 4M3 11V9a3 3 0 0 1 3-3h15M7 22l-4-4 4-4M21 13v2a3 3 0 0 1-3 3H3",
  file: "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6M8 13h8M8 17h5",
};

export type XIconName = keyof typeof PATHS;

export function XIcon(props: { name: XIconName; size?: number; class?: string; style?: JSX.CSSProperties }) {
  const size = () => props.size ?? 18;
  return (
    <svg
      class={props.class}
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.7"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      style={props.style}
    >
      <path d={PATHS[props.name]} />
    </svg>
  );
}
