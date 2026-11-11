// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A view that hears a keyboard paired with or plugged into the phone, around what it holds: what
 * it takes, and what each key sends, are in `src/lib/hardware.ts` (35 §17, slice 9).
 */

import { requireNativeView } from "expo";
import type { NativeSyntheticEvent, ViewProps } from "react-native";

import type { HardwareKey } from "../../src/lib/hardware";

export interface HardwareKeysProps extends ViewProps {
  /** Taking keys at all: while this phone has control. */
  enabled: boolean;
  /** Taking the arrows, Tab and Backspace on their own too: while the field is empty. */
  plain: boolean;
  onKey: (event: NativeSyntheticEvent<HardwareKey>) => void;
}

export const HardwareKeys = requireNativeView<HardwareKeysProps>("HardwareKeys");
