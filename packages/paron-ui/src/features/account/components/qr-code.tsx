import { encode } from "uqr";

export function QrCode({ value, label }: { value: string; label: string }) {
  const { size, data } = encode(value, { border: 2 });
  const modules = data
    .flatMap((row, y) =>
      row.map((dark, x) => (dark ? `M${x} ${y}h1v1h-1z` : "")),
    )
    .join("");

  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${size} ${size}`}
      className="size-48 rounded-lg bg-white"
      shapeRendering="crispEdges"
    >
      <path d={modules} fill="black" />
    </svg>
  );
}
