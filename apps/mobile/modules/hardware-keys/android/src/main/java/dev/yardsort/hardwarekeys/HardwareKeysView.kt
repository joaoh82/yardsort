// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

package dev.yardsort.hardwarekeys

import android.content.Context
import android.view.KeyEvent
import android.view.Window
import android.widget.EditText
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.records.Field
import expo.modules.kotlin.records.Record
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.lang.ref.WeakReference
import java.util.WeakHashMap

class KeyEventRecord(
  @Field val key: String,
  @Field val ctrl: Boolean,
  @Field val alt: Boolean,
  @Field val shift: Boolean,
) : Record

/**
 * Around the composer, the keys a terminal needs from a hardware keyboard. The window hears a key
 * before any view does, so a wrapper around its callback takes them ahead of the focused field,
 * which would otherwise move its cursor with an arrow, and whether or not the field is focused. The
 * screen's own keyboard is left alone: its keys say they are soft.
 *
 * What is taken is kept in step with `takes` in `src/lib/hardware.ts`.
 */
class HardwareKeysView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val onKey by EventDispatcher<KeyEventRecord>()

  private var enabled = false
  /** Whether the arrows, Tab and Backspace on their own are taken: while the field is empty. */
  var plain = false

  /** Keys taken as they went down, so that they are taken as they come up too. */
  private val down = mutableSetOf<Int>()

  /** Whether the Return now held went down in the composer's field. */
  private var enterInField = false

  fun setTaking(enabled: Boolean) {
    this.enabled = enabled
    listen()
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    listen()
  }

  override fun onDetachedFromWindow() {
    super.onDetachedFromWindow()
    KeyCatcher.stop(this)
  }

  private fun listen() {
    val window = appContext.currentActivity?.window
    if (enabled && isAttachedToWindow && window != null) KeyCatcher.start(window, this)
    else KeyCatcher.stop(this)
  }

  /** Whether `event` is this view's to take, and if so, tells the screen on its way down. */
  fun heard(event: KeyEvent): Boolean {
    if (!enabled) return false
    val soft = event.flags and KeyEvent.FLAG_SOFT_KEYBOARD != 0
    // Return goes to the field, which sends what is in it. A text view tells its listener of a
    // keyboard's Return as it goes down and again as it comes up, and React Native's submits each
    // time, so the field would send it twice: it hears only the press. Anything else focused, such
    // as a button reached with Tab, needs the release, which is what presses it.
    if (event.keyCode == KeyEvent.KEYCODE_ENTER && !soft) {
      when (event.action) {
        KeyEvent.ACTION_DOWN -> if (event.repeatCount == 0) enterInField = findFocus() is EditText
        KeyEvent.ACTION_UP -> return enterInField.also { enterInField = false }
      }
      return false
    }
    if (event.action == KeyEvent.ACTION_UP) return down.remove(event.keyCode)
    if (event.action != KeyEvent.ACTION_DOWN) return false
    if (soft || event.isMetaPressed) return false
    val key = nameOf(event) ?: return false
    if (!takes(key, event.isCtrlPressed, event.isAltPressed, event.isShiftPressed)) return false
    down.add(event.keyCode)
    onKey(KeyEventRecord(key, event.isCtrlPressed, event.isAltPressed, event.isShiftPressed))
    return true
  }

  private fun takes(key: String, ctrl: Boolean, alt: Boolean, shift: Boolean): Boolean =
    when (key) {
      "escape" -> true
      "up", "down", "left", "right" -> plain || ctrl || alt || shift
      "tab" -> !ctrl && !alt && (shift || plain)
      "backspace" -> ctrl || alt || plain
      else ->
        key.length == 1 &&
          when {
            ctrl -> CTRL_CHARS.contains(key)
            alt -> ALT_CHARS.contains(key) && (!shift || key[0] in 'a'..'z')
            else -> false
          }
    }

  private fun nameOf(event: KeyEvent): String? =
    when (event.keyCode) {
      KeyEvent.KEYCODE_ESCAPE -> "escape"
      KeyEvent.KEYCODE_TAB -> "tab"
      KeyEvent.KEYCODE_DEL -> "backspace"
      KeyEvent.KEYCODE_DPAD_UP -> "up"
      KeyEvent.KEYCODE_DPAD_DOWN -> "down"
      KeyEvent.KEYCODE_DPAD_LEFT -> "left"
      KeyEvent.KEYCODE_DPAD_RIGHT -> "right"
      else -> {
        // What the key makes with no modifier at all.
        val char = event.getUnicodeChar(0)
        if (char > 0) String(Character.toChars(char)).lowercase() else null
      }
    }

  companion object {
    const val CTRL_CHARS = "abcdefghijklmnopqrstuvwxyz[\\] "
    const val ALT_CHARS = "abcdefghijklmnopqrstuvwxyz0123456789`-=[]\\;',./"
  }
}

/** One wrapper per window, passing each key to the view listening now, if any. */
private class KeyCatcher(private val wrapped: Window.Callback) : Window.Callback by wrapped {
  override fun dispatchKeyEvent(event: KeyEvent?): Boolean {
    val view = target?.get()
    if (event != null && view != null && view.heard(event)) return true
    return wrapped.dispatchKeyEvent(event)
  }

  companion object {
    private var target: WeakReference<HardwareKeysView>? = null
    private val wrapped = WeakHashMap<Window, Unit>()

    fun start(window: Window, view: HardwareKeysView) {
      // Never taken off again: another wrapper may have gone around this one since.
      if (!wrapped.containsKey(window)) {
        window.callback = KeyCatcher(window.callback)
        wrapped[window] = Unit
      }
      target = WeakReference(view)
    }

    fun stop(view: HardwareKeysView) {
      if (target?.get() === view) target = null
    }
  }
}
