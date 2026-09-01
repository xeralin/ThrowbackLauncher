import { Fragment } from "react";
import { ExternalLink } from "@/components/ExternalLink";

const CODE_TOKEN = /(`[^`]+`)/g;
const CODE_EXACT = /^`[^`]+`$/;
const LINK_TOKEN = /(\[[^\]]+\]\((?:[^()\s]|\([^()\s]*\))+\))/g;
const LINK_EXACT = /^\[([^\]]+)\]\(((?:[^()\s]|\([^()\s]*\))+)\)$/;
const EMPHASIS_TOKEN =
  /(\*\*[^\s*](?:[^*]*[^\s*])?\*\*|~~[^\s~](?:[^~]*[^\s~])?~~|\*[^\s*](?:[^*]*[^\s*])?\*|(?<!\w)_[^\s_](?:[^_]*[^\s_])?_(?!\w))/g;
const BOLD_EXACT = /^\*\*[^\s*](?:[^*]*[^\s*])?\*\*$/;
const STRIKE_EXACT = /^~~[^\s~](?:[^~]*[^\s~])?~~$/;
const EM_EXACT = /^\*[^\s*](?:[^*]*[^\s*])?\*$|^_[^\s_](?:[^_]*[^\s_])?_$/;

function renderEmphasis(text: string) {
  return text.split(EMPHASIS_TOKEN).map((part, index) => {
    if (BOLD_EXACT.test(part)) {
      return (
        <strong key={index} className="font-semibold text-text">
          {part.slice(2, -2)}
        </strong>
      );
    }
    if (STRIKE_EXACT.test(part)) {
      return (
        <s key={index} className="opacity-70">
          {part.slice(2, -2)}
        </s>
      );
    }
    if (EM_EXACT.test(part)) {
      return <em key={index}>{part.slice(1, -1)}</em>;
    }
    return part;
  });
}

function renderLinks(text: string) {
  return text.split(LINK_TOKEN).map((part, index) => {
    const link = LINK_EXACT.exec(part);
    if (link) {
      return (
        <ExternalLink
          key={index}
          href={link[2]}
          className="text-link hover:underline"
        >
          {link[1]}
        </ExternalLink>
      );
    }
    return <Fragment key={index}>{renderEmphasis(part)}</Fragment>;
  });
}

export function renderInline(text: string) {
  return text.split(CODE_TOKEN).map((part, index) => {
    if (CODE_EXACT.test(part)) {
      return <code key={index}>{part.slice(1, -1)}</code>;
    }
    return <Fragment key={index}>{renderLinks(part)}</Fragment>;
  });
}
