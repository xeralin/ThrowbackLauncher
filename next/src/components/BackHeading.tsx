import { StrokeIcon, CHEVRON_LEFT } from "@/components/icons";

export function BackHeading({
  title,
  onBack,
}: {
  title: string;
  onBack: () => void;
}) {
  return (
    <h1 className="mb-4 font-display text-[1.9rem] font-bold text-text">
      <button
        type="button"
        autoFocus
        onClick={onBack}
        className="group flex w-fit items-center"
      >
        <StrokeIcon
          d={CHEVRON_LEFT}
          className="-ml-2 size-6 shrink-0 -translate-y-[1.5px] text-text-muted transition-colors group-hover:text-text"
        />
        <span className="sr-only">Back </span>
        <span>{title}</span>
      </button>
    </h1>
  );
}
