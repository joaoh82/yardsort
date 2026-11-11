// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Phone notifications. The server chooses the words. A one-time code is never in the payload,
 * and this client has no way to answer or approve from the notice itself.
 */

import { bool, DecodeError, integer, object, text, type Decoder } from "./decode.ts";

export type NoticeCategory = "task" | "review" | "failure" | "desktopUpdate" | "workflow";

export interface NotificationPreferences {
  muted: boolean;
  task: boolean;
  review: boolean;
  failure: boolean;
  desktopUpdate: boolean;
  workflow: boolean;
}

export interface PushRegistration {
  platform: "apns" | "fcm";
  token: string;
}

export interface NoticeView {
  id: string;
  category: NoticeCategory;
  kind: string;
  title: string;
  body: string;
  deepLink: string;
}

export interface ReachableReceipt {
  sent: number;
}

const category: Decoder<NoticeCategory> = (value, path) => {
  const name = text(32)(value, path);
  if (
    name === "task" ||
    name === "review" ||
    name === "failure" ||
    name === "desktopUpdate" ||
    name === "workflow"
  ) {
    return name;
  }
  throw new DecodeError(path, "is not one this build knows");
};

export const notificationPreferences: Decoder<NotificationPreferences> = object({
  muted: bool,
  task: bool,
  review: bool,
  failure: bool,
  desktopUpdate: bool,
  workflow: bool,
});

export const noticeView: Decoder<NoticeView> = object({
  id: text(80),
  category,
  kind: text(32),
  title: text(200),
  body: text(500),
  deepLink: text(300),
});

export const reachableReceipt: Decoder<ReachableReceipt> = object({
  sent: integer(0, 1_000_000),
});
