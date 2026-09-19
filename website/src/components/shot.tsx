import fs from "node:fs";
import path from "node:path";

// A framed screenshot from docs/images. Width and height are read from the PNG header at build
// time so the page never shifts while images load. `crop` shows a region of the image in a 4:3
// frame: the design reuses the overview screenshot for its terminal and changes panels.
type Props = { name: string; alt: string; crop?: string; priority?: boolean; className?: string };

function pngSize(name: string): { width: number; height: number } {
  const file = path.resolve(process.cwd(), "..", "docs", "images", `${name}.png`);
  const header = Buffer.alloc(24);
  const fd = fs.openSync(file, "r");
  fs.readSync(fd, header, 0, 24, 0);
  fs.closeSync(fd);
  return { width: header.readUInt32BE(16), height: header.readUInt32BE(20) };
}

export function Shot({ name, alt, crop, priority, className = "" }: Props) {
  const { width, height } = pngSize(name);
  return (
    <div
      className={`overflow-hidden rounded-[10px] border border-line bg-surface ${crop ? "aspect-[4/3]" : ""} ${className}`}
    >
      {/* eslint-disable-next-line @next/next/no-img-element -- static export, images are plain files */}
      <img
        src={`/docs-images/${name}.png`}
        alt={alt}
        width={width}
        height={height}
        loading={priority ? "eager" : "lazy"}
        fetchPriority={priority ? "high" : undefined}
        className={crop ? "block h-full w-full object-cover" : "block h-auto w-full"}
        style={crop ? { objectPosition: crop } : undefined}
      />
    </div>
  );
}
