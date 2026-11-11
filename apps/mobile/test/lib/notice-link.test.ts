// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import { noticeRoute } from "../../src/lib/notice-link";

const ID = "11111111-1111-4111-8111-111111111111";

describe("a notification link", () => {
  test("opens the notice", () => {
    expect(noticeRoute(`yardsort://notice/workflow/${ID}`)).toBe(`/notice/${ID}`);
    expect(noticeRoute(`yardsort:///notice/desktop-update/${ID}`)).toBe(`/notice/${ID}`);
  });

  test("leaves a pairing link and a sign-in return alone", () => {
    expect(noticeRoute("yardsort://pair/abc")).toBeNull();
    expect(noticeRoute("yardsort://signed-in?code=secret")).toBeNull();
    expect(noticeRoute(`yardsort://notice/workflow/${ID}?code=Q7K2M9NP`)).toBeNull();
  });
});
