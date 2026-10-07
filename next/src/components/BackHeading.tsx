import { StrokeIcon, CHEVRON_LEFT } from "@/components/icons";
import { pageTitle } from "@/components/ui";

export function BackHeading({
  title,
  onBack,
}: {
  title: string;
  onBack: () => void;
}) {
  return (
    <h1 className={`mb-4 ${pageTitle} leading-none`}>
      <button
        type="button"
        autoFocus
        onClick={onBack}
        className="group flex w-fit items-center"
      >
        <StrokeIcon
          d={CHEVRON_LEFT}
          className="-ml-2 size-6 shrink-0 -translate-y-[2.5px] text-text-muted transition-colors group-hover:text-text"
        />
        <span className="sr-only">Back </span>
        <span>{title}</span>
      </button>
    </h1>
  );
}
