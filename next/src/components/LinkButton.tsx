import type { ReactNode } from "react";
import { buttonBase, buttonVariants } from "./Button";
import { ExternalLink } from "./ExternalLink";

export function LinkButton({
  href,
  variant,
  download,
  children,
}: {
  href: string;
  variant: "primary" | "secondary";
  download?: string;
  children: ReactNode;
}) {
  const className = `${buttonBase} ${buttonVariants[variant]} no-underline`;
  if (download) {
    return (
      <a href={href} download={download} className={className}>
        {children}
      </a>
    );
  }
  return (
    <ExternalLink href={href} className={className}>
      {children}
    </ExternalLink>
  );
}
