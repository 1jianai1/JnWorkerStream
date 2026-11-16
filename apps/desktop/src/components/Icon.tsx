import type { SVGProps } from "react";

export type IconName =
  | "link"
  | "activity"
  | "terminal"
  | "wave"
  | "keyboard"
  | "settings"
  | "sparkles"
  | "zap"
  | "play"
  | "pause"
  | "download"
  | "search"
  | "chevronDown"
  | "chevronRight"
  | "alertTriangle"
  | "check"
  | "x"
  | "copy"
  | "refresh"
  | "usb"
  | "bluetooth"
  | "radio"
  | "arrowDown"
  | "arrowRight"
  | "clock"
  | "cpu"
  | "folder"
  | "file"
  | "plus"
  | "trash"
  | "grip"
  | "rotate"
  | "cube"
  | "grid"
  | "plug"
  | "info"
  | "external"
  | "sliders"
  | "shield"
  | "monitor"
  | "power"
  | "wifi";

interface IconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  name: IconName;
  size?: number;
}

const paths: Partial<Record<IconName, string[]>> = {
  link: [
    "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71",
    "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71",
  ],
  activity: ["M22 12h-4l-3 9L9 3l-3 9H2"],
  terminal: ["M4 17l6-6-6-6", "M12 19h8"],
  wave: ["M2 12h2l2-6 3 12 3-9 3 6 2-3h3"],
  keyboard: ["M4 5h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2Z", "M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M18 13h.01M7 16h10"],
  settings: [
    "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z",
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z",
  ],
  sparkles: ["M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3Z", "M19 15l.9 2.1L22 18l-2.1.9L19 21l-.9-2.1L16 18l2.1-.9L19 15Z"],
  zap: ["M13 2 3 14h9l-1 8 10-12h-9l1-8Z"],
  play: ["M6 4l14 8-14 8V4Z"],
  pause: ["M8 4v16M16 4v16"],
  download: ["M12 3v12", "M7 10l5 5 5-5", "M4 21h16"],
  search: ["M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16Z", "M21 21l-4.3-4.3"],
  chevronDown: ["M6 9l6 6 6-6"],
  chevronRight: ["M9 6l6 6-6 6"],
  alertTriangle: ["M10.3 3.6 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.6a2 2 0 0 0-3.4 0Z", "M12 9v4M12 17h.01"],
  check: ["M20 6 9 17l-5-5"],
  x: ["M18 6 6 18M6 6l12 12"],
  copy: ["M8 8h12v12H8z", "M16 8V4H4v12h4"],
  refresh: ["M21 12a9 9 0 1 1-2.6-6.4", "M21 3v6h-6"],
  usb: ["M12 21V7", "M12 7l-3-3M12 7l3-3", "M9 13l-4-2v4l4-2ZM15 11l4 2-4 2v-4Z", "M12 3v2"],
  bluetooth: ["M7 7l10 10-5 4V3l5 4L7 17"],
  radio: ["M4 9h16v11H4z", "M8 15h.01M12 15h.01M16 15h.01", "M7 9 17 3"],
  arrowDown: ["M12 4v16M6 14l6 6 6-6"],
  arrowRight: ["M4 12h16M14 6l6 6-6 6"],
  clock: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z", "M12 7v5l3 2"],
  cpu: ["M6 6h12v12H6z", "M9 9h6v6H9z", "M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"],
  folder: ["M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"],
  file: ["M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9l-6-6Z", "M14 3v6h6"],
  plus: ["M12 5v14M5 12h14"],
  trash: ["M4 7h16", "M9 7V4h6v3", "M6 7l1 14h10l1-14", "M10 11v6M14 11v6"],
  grip: ["M9 5h.01M9 12h.01M9 19h.01M15 5h.01M15 12h.01M15 19h.01"],
  rotate: ["M3 12a9 9 0 1 0 3-6.7", "M3 4v5h5"],
  cube: ["M12 2 3 7v10l9 5 9-5V7l-9-5Z", "M3 7l9 5 9-5", "M12 12v10"],
  grid: ["M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z"],
  plug: ["M9 2v6M15 2v6", "M6 8h12v3a6 6 0 0 1-12 0V8Z", "M12 17v5"],
  info: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z", "M12 11v5M12 8h.01"],
  external: ["M14 3h7v7", "M10 14 21 3", "M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5"],
  sliders: ["M4 6h16M4 12h16M4 18h16", "M8 3v6M16 9v6M10 15v6"],
  shield: ["M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z", "M9 12l2 2 4-4"],
  monitor: ["M3 4h18v12H3z", "M8 20h8M12 16v4"],
  power: ["M12 2v10", "M18.4 6.6a9 9 0 1 1-12.8 0"],
  wifi: ["M2 8a15 15 0 0 1 20 0", "M5 11a10 10 0 0 1 14 0", "M8.5 14a5 5 0 0 1 7 0", "M12 18h.01"],
};

export function Icon({ name, size = 18, ...rest }: IconProps) {
  const data = paths[name];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {data?.map((path) => <path key={path} d={path} />)}
    </svg>
  );
}
