// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What an agent wrote, drawn at the phone's width (43). Everything wraps, code too: nothing here
 * scrolls sideways. A link opens in the system's browser after the person has seen where it leads.
 */

import { Alert, Linking, StyleSheet, Text, View } from "react-native";

import { type Block, type Span, readMarkdown } from "../lib/markdown";
import { FONTS, useColors } from "./parts";

export function Markdown({ text, muted }: { text: string; muted?: boolean }) {
  const blocks = readMarkdown(text);
  return (
    <View style={styles.blocks}>
      {blocks.map((block, i) => (
        <BlockView key={i} block={block} muted={muted ?? false} />
      ))}
    </View>
  );
}

function BlockView({ block, muted }: { block: Block; muted: boolean }) {
  const colors = useColors();
  const ink = muted ? colors.muted : colors.ink;
  switch (block.kind) {
    case "paragraph":
      return (
        <Text style={[styles.body, { color: ink }]}>
          <Spans spans={block.spans} />
        </Text>
      );
    case "heading":
      return (
        <Text
          accessibilityRole="header"
          style={[block.level <= 2 ? styles.heading : styles.subheading, { color: ink }]}
        >
          <Spans spans={block.spans} />
        </Text>
      );
    case "list":
      return (
        <View style={styles.list}>
          {block.items.map((item, i) => (
            <View key={i} style={styles.item}>
              <Text style={[styles.marker, { color: colors.faint }]}>
                {block.ordered ? `${block.start + i}.` : "•"}
              </Text>
              <Text style={[styles.body, styles.itemText, { color: ink }]}>
                <Spans spans={item} />
              </Text>
            </View>
          ))}
        </View>
      );
    case "code":
      return (
        <View
          style={[styles.code, { backgroundColor: colors.raised, borderColor: colors.line }]}
          accessibilityLabel="Code"
        >
          <Text selectable style={[styles.codeText, { color: colors.ink }]}>
            {block.text}
          </Text>
        </View>
      );
    case "quote":
      return (
        <View style={[styles.quote, { borderColor: colors.lineStrong }]}>
          <Text style={[styles.body, { color: colors.muted }]}>
            <Spans spans={block.spans} />
          </Text>
        </View>
      );
    case "rule":
      return <View style={[styles.rule, { backgroundColor: colors.line }]} />;
    case "table":
      // A table as wide as a computer's screen cannot be a table on a phone: each row is a card
      // of its cells, each under its column's name.
      return (
        <View style={styles.table}>
          {block.rows.map((row, r) => (
            <View
              key={r}
              style={[
                styles.tableRow,
                { borderColor: colors.line, backgroundColor: colors.surface },
              ]}
            >
              {row.map((cell, c) => (
                <Text key={c} style={[styles.cell, { color: ink }]}>
                  {block.header[c] && block.header[c].length > 0 ? (
                    <Text style={[styles.cellName, { color: colors.faint }]}>
                      {block.header[c].map((s) => s.text).join("")}
                      {"  "}
                    </Text>
                  ) : null}
                  <Spans spans={cell} />
                </Text>
              ))}
            </View>
          ))}
        </View>
      );
  }
}

function Spans({ spans }: { spans: Span[] }) {
  const colors = useColors();
  return (
    <>
      {spans.map((span, i) => {
        if (span.code) {
          return (
            <Text key={i} style={[styles.inlineCode, { color: colors.accentText }]}>
              {span.text}
            </Text>
          );
        }
        if (span.href) {
          const href = span.href;
          return (
            <Text
              key={i}
              accessibilityRole="link"
              onPress={() => confirmLink(href)}
              style={[styles.link, { color: colors.accentText }]}
            >
              {span.text}
            </Text>
          );
        }
        return (
          <Text key={i} style={span.bold ? styles.bold : span.italic ? styles.italic : undefined}>
            {span.text}
          </Text>
        );
      })}
    </>
  );
}

function confirmLink(url: string): void {
  if (!/^https?:\/\//i.test(url)) return;
  Alert.alert("Open this link?", url, [
    { text: "Cancel", style: "cancel" },
    { text: "Open", onPress: () => void Linking.openURL(url).catch(() => {}) },
  ]);
}

const styles = StyleSheet.create({
  blocks: { gap: 10 },
  body: { fontFamily: FONTS.regular, fontSize: 15, lineHeight: 23 },
  heading: { fontFamily: FONTS.semibold, fontSize: 17, lineHeight: 24 },
  subheading: { fontFamily: FONTS.semibold, fontSize: 15, lineHeight: 22 },
  bold: { fontFamily: FONTS.semibold },
  italic: { fontStyle: "italic" },
  link: { textDecorationLine: "underline" },
  inlineCode: { fontFamily: FONTS.mono, fontSize: 14 },
  list: { gap: 6 },
  item: { flexDirection: "row", gap: 8 },
  marker: { fontFamily: FONTS.regular, fontSize: 15, lineHeight: 23, minWidth: 14 },
  itemText: { flex: 1 },
  code: {
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  codeText: { fontFamily: FONTS.mono, fontSize: 13, lineHeight: 20 },
  quote: { borderLeftWidth: 3, paddingLeft: 10 },
  rule: { height: StyleSheet.hairlineWidth },
  table: { gap: 6 },
  tableRow: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    gap: 2,
  },
  cell: { fontFamily: FONTS.regular, fontSize: 14, lineHeight: 20 },
  cellName: { fontFamily: FONTS.mono, fontSize: 11 },
});
