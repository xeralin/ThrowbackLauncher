export function VersionChip({
  version,
  className = "",
}: {
  version: string;
  className?: string;
}) {
  return (
    <code className={`chip ${className}`}>v{version.replace(/^v/, "")}</code>
  );
}
