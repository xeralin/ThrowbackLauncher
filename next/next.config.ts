import type { NextConfig } from "next";
import keyArtWidths from "./src/config/keyart-widths.json";

const ONE_YEAR_SECONDS = 365 * 24 * 60 * 60;

const nextConfig: NextConfig = {
  output: "export",
  agentRules: false,
  trailingSlash: true,
  experimental: {
    staleTimes: {
      static: ONE_YEAR_SECONDS,
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
