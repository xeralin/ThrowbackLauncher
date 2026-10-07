import type { ReactNode } from "react";
import { buttonBase, buttonVariants } from "@/components/Button";
import { ExternalLink } from "@/components/ExternalLink";

export function LinkButton({
  href,
  variant,
  children,
}: {
  href: string;
  variant: "primary" | "secondary";
  children: ReactNode;
}) {
  return (
    <ExternalLink
      href={href}
      className={`${buttonBase} ${buttonVariants[variant]} no-underline`}
    >
      {children}
    </ExternalLink>
  );
}
