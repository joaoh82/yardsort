import path from "node:path";
import type { NextConfig } from "next";

// A fully static site: `next build` writes plain files to out/, which any static host can serve.
const nextConfig: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  // The site lives in a subfolder of the app's repository and reads ../docs.
  turbopack: { root: path.join(__dirname, "..") },
};

export default nextConfig;
