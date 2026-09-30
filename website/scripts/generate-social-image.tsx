// Run with `bun run social-image`. Commit the PNG; builds do not regenerate it, so a docs
// screenshot update cannot silently change the card. No network or runtime image service.
import fs from "node:fs/promises";
import path from "node:path";
import { ImageResponse } from "next/og";

const root = path.resolve(import.meta.dirname, "..");
const logo = await fs.readFile(path.join(root, "public/icon.svg"));
const screenshot = await fs.readFile(path.join(root, "../docs/images/overview.png"));
const response = new ImageResponse(
  <div
    style={{
      display: "flex",
      width: "100%",
      height: "100%",
      background: "#0f1115",
      color: "#e8eaed",
      position: "relative",
      fontFamily: "sans-serif",
    }}
  >
    <div
      style={{
        display: "flex",
        position: "absolute",
        left: 56,
        top: 48,
        alignItems: "center",
        gap: 18,
      }}
    >
      {/* eslint-disable-next-line @next/next/no-img-element -- rendered by Satori, not a browser */}
      <img
        src={`data:image/svg+xml;base64,${logo.toString("base64")}`}
        width={70}
        height={70}
        alt=""
      />
      <span style={{ fontSize: 42, fontWeight: 700, letterSpacing: -1 }}>Yardsort</span>
    </div>
    <div
      style={{
        display: "flex",
        position: "absolute",
        left: 64,
        top: 176,
        width: 570,
        flexDirection: "column",
        fontSize: 64,
        fontWeight: 700,
        lineHeight: 1.08,
        letterSpacing: -2,
      }}
    >
      <span>Run AI coding</span>
      <span>agents in</span>
      <span style={{ color: "#f5b83d" }}>parallel.</span>
    </div>
    <div
      style={{
        display: "flex",
        position: "absolute",
        left: 64,
        top: 418,
        flexDirection: "column",
        fontSize: 25,
        lineHeight: 1.4,
        color: "#9aa3af",
      }}
    >
      <span>One task. One git worktree.</span>
      <span>Your choice of agent.</span>
    </div>
    <div
      style={{
        display: "flex",
        position: "absolute",
        left: 640,
        top: 176,
        width: 510,
        height: 320,
        border: "2px solid #303641",
        borderRadius: 12,
        overflow: "hidden",
      }}
    >
      {/* eslint-disable-next-line @next/next/no-img-element -- embedded demo screenshot */}
      <img
        src={`data:image/png;base64,${screenshot.toString("base64")}`}
        width={510}
        height={320}
        alt="Yardsort workspaces, agent terminal and code diff"
      />
    </div>
    <div
      style={{
        display: "flex",
        position: "absolute",
        left: 64,
        right: 56,
        bottom: 40,
        paddingTop: 22,
        borderTop: "1px solid #262b34",
        justifyContent: "space-between",
        fontSize: 22,
        color: "#9aa3af",
      }}
    >
      <span>Linux · macOS · Windows</span>
      <span style={{ color: "#f5b83d" }}>yardsort.sh</span>
    </div>
  </div>,
  { width: 1200, height: 630 },
);

const output = path.join(root, "public/social/yardsort-v1.png");
await fs.mkdir(path.dirname(output), { recursive: true });
await fs.writeFile(output, Buffer.from(await response.arrayBuffer()));
console.log(`Social card: ${output}`);
