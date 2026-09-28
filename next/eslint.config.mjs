import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  {
    files: ["src/components/Markdown.tsx", "src/components/NyanCat.tsx"],
    rules: { "@next/next/no-img-element": "off" },
  },
  globalIgnores(["public/qwebchannel.js"]),
]);

export default eslintConfig;
