// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, jest, test } from "@jest/globals";
import { fireEvent, render, screen } from "@testing-library/react-native";

import { OFFLINE } from "../../src/lib/workflows";
import { DecisionForm } from "../../src/ui/DecisionForm";

describe("a workflow decision", () => {
  test("does not send while the computer is offline", async () => {
    const decide = jest.fn();
    await render(
      <DecisionForm
        online={false}
        kind="approval"
        locked={false}
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
});
