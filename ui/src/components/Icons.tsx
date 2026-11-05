// Minimal inline icon set (stroke icons, 16px grid).
const base = {
  width: 16,
  height: 16,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.4,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export const BranchIcon = () => (
  <svg {...base}>
    <circle cx="4.5" cy="3.5" r="1.6" />
    <circle cx="4.5" cy="12.5" r="1.6" />
    <circle cx="11.5" cy="5" r="1.6" />
    <path d="M4.5 5.1v5.8M11.5 6.6c0 2.6-4.6 2.2-6.6 4.4" />
  </svg>
);
export const FetchIcon = () => (
  <svg {...base}>
    <path d="M8 2.5v8M4.8 7.5 8 10.7l3.2-3.2M3 13.5h10" />
  </svg>
);
export const PullIcon = () => (
  <svg {...base}>
    <path d="M8 2.5v9M4.5 8 8 11.5 11.5 8" />
  </svg>
);
export const PushIcon = () => (
  <svg {...base}>
    <path d="M8 13.5v-9M4.5 8 8 4.5 11.5 8" />
  </svg>
);
export const UndoIcon = () => (
  <svg {...base}>
    <path d="M5.5 3.5 2.5 6.5l3 3M2.8 6.5H10a3.5 3.5 0 0 1 0 7H7" />
  </svg>
);
export const RefreshIcon = () => (
  <svg {...base}>
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.8v2.6h-2.6" />
  </svg>
);
export const FolderIcon = () => (
  <svg {...base}>
    <path d="M2 4.5c0-.6.4-1 1-1h3l1.5 1.5H13c.6 0 1 .4 1 1v6c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1z" />
  </svg>
);
export const ChevronDown = () => (
  <svg {...base} width={12} height={12}>
    <path d="M4 6l4 4 4-4" />
  </svg>
);
export const CheckIcon = () => (
  <svg {...base} width={14} height={14}>
    <path d="M3.5 8.5 6.5 11.5 12.5 4.5" />
  </svg>
);
