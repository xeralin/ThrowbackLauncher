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
    description: "What to include in a report so the staff can help you.",
    tag: "Support & Troubleshooting",
    corner: "HELP",
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
