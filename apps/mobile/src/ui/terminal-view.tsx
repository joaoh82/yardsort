// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The WebView a terminal is drawn in (31 §5). It loads one document, shipped with the app. It goes
 * nowhere else, keeps nothing, and says only what `lib/bridge.ts` lets it say. A link goes to the
 * system's browser, and only after the person has seen where it leads: a link's text in a terminal
 * can say anything.
 */

import { useEffect, useRef, useState } from "react";
import { Alert, Linking, StyleSheet } from "react-native";
import { WebView } from "react-native-webview";

import { readToApp } from "../lib/bridge";
import { pageSaid, type PageFeed } from "../lib/watch";
import { TERMINAL_PAGE } from "./terminal-page.generated";

const BLANK = "about:blank";

export function TerminalView({
  feed,
  started,
  trouble,
}: {
  feed: PageFeed;
  /** The page has started, with nothing on it: the first time, and whenever it is started again. */
  started: () => void;
  trouble: (what: "script" | "unreadable") => void;
}) {
  const web = useRef<WebView<object>>(null);
  // The system may end a WebView's process to free memory. A new view starts it again.
  const [life, setLife] = useState(0);
  useEffect(() => () => feed.connect(null), [feed]);

  const lost = () => {
    feed.connect(null);
    setLife((n) => n + 1);
  };

  return (
    <WebView<object>
      key={life}
      ref={web}
      testID="terminal"
      accessibilityLabel="Terminal, drawn at the computer’s size"
      style={styles.fill}
      source={{ html: TERMINAL_PAGE, baseUrl: BLANK }}
      originWhitelist={[BLANK]}
      onShouldStartLoadWithRequest={(request) => request.url === BLANK}
      onMessage={(event) => {
        const message = readToApp(event.nativeEvent.data);
        if (!message) return;
        const left = pageSaid(message, feed);
        if (left?.t === "ready") {
          feed.connect((text) => web.current?.postMessage(text));
          started();
        } else if (left?.t === "link") {
          confirmLink(left.url);
        } else if (left?.t === "trouble") {
          trouble(left.what);
        }
      }}
      onContentProcessDidTerminate={lost}
      onRenderProcessGone={lost}
      contentInsetAdjustmentBehavior="automatic"
      bounces={false}
      javaScriptEnabled
      javaScriptCanOpenWindowsAutomatically={false}
      setSupportMultipleWindows={false}
      domStorageEnabled={false}
      cacheEnabled={false}
      incognito
      thirdPartyCookiesEnabled={false}
      sharedCookiesEnabled={false}
      allowFileAccess={false}
      allowFileAccessFromFileURLs={false}
      allowUniversalAccessFromFileURLs={false}
      allowsLinkPreview={false}
      geolocationEnabled={false}
      mediaCapturePermissionGrantType="deny"
      dataDetectorTypes="none"
      saveFormDataDisabled
      setBuiltInZoomControls
      setDisplayZoomControls={false}
      hideKeyboardAccessoryView
    />
  );
}

function confirmLink(url: string): void {
  Alert.alert("Open this link?", url, [
    { text: "Cancel", style: "cancel" },
    { text: "Open", onPress: () => void Linking.openURL(url).catch(() => {}) },
  ]);
}

const styles = StyleSheet.create({ fill: { flex: 1, backgroundColor: "#151514" } });
