type FaqPage = {
  title: string;
  description: string;
  tag: string;
  corner: string;
};

export const FAQ_PAGES = {
  "getting-started": {
    title: "Getting Started",
    description: "How to set up and download Operation Throwback.",
    tag: "Support",
    corner: "SETUP",
  },
  "common-errors": {
    title: "Common Errors",
    description: "Solutions to the most frequently encountered game issues.",
    tag: "Support",
    corner: "ERR",
  },
  multiplayer: {
    title: "Multiplayer",
    description: "How to set up and play with others using Radmin VPN.",
    tag: "Support",
    corner: "MP",
  },
  "how-to-get-help": {
    title: "How to Get Help",
    description: "What to include in a report so the staff can help you.",
    tag: "Support",
    corner: "HELP",
  },
  "heated-metal": {
    title: "Heated Metal",
    description: "An SDK with a map editor, scripting, unlock all and more.",
    tag: "Tools & Mods",
    corner: "HM",
  },
} satisfies Record<string, FaqPage>;
