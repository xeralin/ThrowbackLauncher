import type { NextConfig } from "next";
import keyArtWidths from "./src/config/keyart-widths.json";

const nextConfig: NextConfig = {
  output: "export",
  agentRules: false,
  trailingSlash: true,
  experimental: {
    staleTimes: {
      static: 365 * 24 * 60 * 60,
    },
  },
  images: {
    loader: "custom",
    loaderFile: "./src/lib/image-loader.ts",
    deviceSizes: keyArtWidths,
    imageSizes: [],
  },
};

export default nextConfig;
