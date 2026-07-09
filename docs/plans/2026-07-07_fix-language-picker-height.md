# Fix Language Picker Height in Settings Modal

## Context
The config modal in `src/App.tsx` has a `<select>` element for language selection ("Idioma / Language") that renders at a different intrinsic height than the adjacent `<input type="text">` elements (API Key, Base URL, Model fields). 

Both the `<select>` and `<input>` elements currently share the same classes: `p-2 text-sm`. However, browsers render native `<select>` elements with OS-level chrome (the dropdown arrow button) that changes the intrinsic height, making the language picker slightly shorter/taller than the text inputs.

## Root Cause
Native `<select>` elements have `appearance: auto` by default in most browsers, which gives them OS-native styling including a different box model height than `<input type="text">`.

## Solution
1. Add `appearance-none` to the `<select>` element class list to strip native OS styling
2. Keep the same `p-2 text-sm` classes so it matches inputs exactly

**File:** `src/App.tsx` ~line 216
**Change:** Add `appearance-none` to the `<select>` class string.

## Risks
- Minimal. The select will lose its native dropdown arrow affordance, but Tailwind's `bg-surface-0` background + border already provide enough visual context. If desired, a custom dropdown arrow could be added via a pseudo-element, but that's beyond the scope of this fix.

## Verification
- Open the config modal and visually compare language picker height with the text input fields below it
