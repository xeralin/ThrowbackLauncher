import { Hero } from "@/components/Hero";
import { FAQ_PAGES } from "@/config/faq";

export function FaqHero({
  page,
}: {
  page: Exclude<keyof typeof FAQ_PAGES, "index">;
}) {
  const { title, description, tag, corner } = FAQ_PAGES[page];
  return (
    <Hero
      tag={tag}
      corner={corner}
      title={<em>{title}</em>}
      description={description}
    />
  );
}
