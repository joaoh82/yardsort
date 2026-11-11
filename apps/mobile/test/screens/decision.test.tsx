// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, jest, test } from "@jest/globals";
import { ClientError } from "@yardsort/remote-client";
import { act, fireEvent, render, screen } from "@testing-library/react-native";

import { OFFLINE } from "../../src/lib/workflows";
import { DecisionForm } from "../../src/ui/DecisionForm";
import { challenge, reads } from "./decision-harness";

jest.mock("expo-crypto", () => ({ randomUUID: () => "decision-id" }));
jest.mock("expo-router", () => ({
  Redirect: () => null,
  Stack: { Screen: () => null },
  useLocalSearchParams: () => ({ host: "host-1", run: "run-1", step: "sign-off" }),
}));
jest.mock("../../src/ui/live", () => {
  // The factory runs before the file's imports, so the harness is loaded here.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const harness = require("./decision-harness") as typeof import("./decision-harness");
  return {
    OfProfile: ({ children }: { children: unknown }) => children,
    ReachLine: () => null,
    useChannelTo: () => ({
      reach: "online",
      channel: {
        workflowChallenge: (...args: unknown[]) => harness.challenge(...args),
        workflowDecide: async () => {
          throw new Error("the decision was sent");
        },
        workflowRun: async () => harness.detail,
      },
    }),
    useRead: (_channel: unknown, _read: unknown, asked: number) => {
      harness.reads.push(asked);
      return { answer: harness.detail, problem: null };
    },
  };
});

// Loaded after the mocks above, which replace the modules this screen imports.
// eslint-disable-next-line import/first
import Decision from "../../src/app/decision";

describe("a workflow decision", () => {
  test("does not send while the computer is offline", async () => {
    const decide = jest.fn();
    await render(
      <DecisionForm
        online={false}
        kind="approval"
        locked={false}
        resumeEligible={false}
        busy={false}
        note=""
        onNote={() => {}}
        onDecide={decide}
      />,
    );
    expect(screen.getByText(OFFLINE)).toBeTruthy();
    fireEvent.press(screen.getByLabelText("Approve"));
    expect(decide).not.toHaveBeenCalled();
  });

  test("approves once while the computer is online", async () => {
    const decide = jest.fn();
    await render(
      <DecisionForm
        online
        kind="approval"
        locked={false}
        resumeEligible={false}
        busy={false}
        note=""
        onNote={() => {}}
        onDecide={decide}
      />,
    );
    fireEvent.press(screen.getByLabelText("Approve"));
    expect(decide).toHaveBeenCalledTimes(1);
    expect(decide).toHaveBeenCalledWith("approve");
  });

  test("a stale challenge reloads the run and does not decide", async () => {
    reads.length = 0;
    challenge.mockRejectedValue(
      ClientError.from({ code: "stale", message: "The step moved.", retryable: false }),
    );
    await render(<Decision />);
    await act(async () => {
      fireEvent.press(screen.getByLabelText("Approve"));
    });
    expect(await screen.findByText("This changed. Refresh and read it again.")).toBeTruthy();
    expect(reads).toContain(1);
    expect(challenge).toHaveBeenCalled();
  });

  test("the old refresh sentence on another error does not reload the run", async () => {
    reads.length = 0;
    challenge.mockRejectedValue(
      ClientError.from({
        code: "invalid_request",
        message: "This changed. Refresh and read it again.",
        retryable: false,
      }),
    );
    await render(<Decision />);
    await act(async () => {
      fireEvent.press(screen.getByLabelText("Approve"));
    });
    expect(await screen.findByText("This changed. Refresh and read it again.")).toBeTruthy();
    expect(reads.every((asked) => asked === 0)).toBe(true);
  });
});
