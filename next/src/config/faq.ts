type FaqPage = {
  title: string;
  description: string;
  tag?: string;
  corner?: string;
};

export const FAQ_PAGES = {
  index: {
    title: "FAQ",
    description:
      "Your guide to downloading, setting up, and playing older Rainbow Six Siege seasons.",
  },
  general: {
    title: "General",
    description: "Common questions about setting up and using the Launcher.",
    tag: "Support & Troubleshooting",
    corner: "GEN",
  },
  multiplayer: {
    title: "Multiplayer",
    description:
      "How to set up and play with others using Radmin VPN or ZeroTier.",
    tag: "Support & Troubleshooting",
    corner: "MP",
  },
  "common-errors": {
    title: "Common Errors",
    description: "Solutions to the most frequently encountered game issues.",
    tag: "Support & Troubleshooting",
    corner: "ERR",
  },
  "how-to-get-help": {
    title: "How to Get Help",
    description:
      "Cannot find an answer in the FAQ? Here is how to get support from the community and staff.",
    tag: "Support & Troubleshooting",
    corner: "HELP",
  },
  liberator: {
    title: "Liberator",
    description:
      "Unlock all cosmetics and play additional game modes in older Rainbow Six Siege seasons.",
    tag: "Tools & Mods",
    corner: "LIB",
  },
  "heated-metal": {
    title: "Heated Metal",
    description:
      "An SDK for Rainbow Six Siege — map editor, extended scripting, unlock all, and more.",
    tag: "Tools & Mods",
    corner: "HM",
  },
  "cheat-engine": {
    title: "Cheat Engine",
    description: "How to use Cheat Engine to modify old Rainbow Six Siege.",
    tag: "Tools & Mods",
    corner: "CE",
  },
} satisfies Record<string, FaqPage>;
