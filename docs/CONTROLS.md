# Keyboard, mouse and Xbox 360 controls

Both input methods work without changing legacy saves or key bindings. No
controller is required. Mouse movement is relative/recentered during gameplay
on Windows; menus use a normal pointer. Losing focus releases inputs and cursor
confinement, and pauses simulation. Controller disconnection leaves keyboard
and mouse usable. Look Speed and Invert Look apply to both input methods.
Windows pointer speed/acceleration also affects this recentered mouse input.

| Command | Keyboard/mouse | Xbox 360 |
|---|---|---|
| Move/strafe | WASD | Left stick |
| Look | Mouse | Right stick |
| Run | Hold Shift | Hold L3 |
| Jump, including from rest | Space | A |
| Crouch | Ctrl | B |
| Fire/cast | Left mouse | RT |
| Hand aiming/interaction mode | Hold right mouse | Hold LT |
| Door/switch/enter vehicle | E | X |
| Weapons | 1 | LB |
| Darklight powers | 2 | RB |
| Inventory/keys | 3 or I | Y |
| Center look | R | R3 |
| Fly up/down | Page Up/Page Down | D-pad up/down |
| VDU | Q | D-pad left |
| Land/exit vehicle | F | D-pad right |
| Automap | M or Tab | Back -> Automap |
| Original menu | Esc | Start |
| Actions panel | F4 | Back |
| Control guides | F1 or Actions -> Control Guides | Back -> Control Guides |
| Pause without menus | P | Actions -> Pause |
| Skip NPC speech | Backslash | Actions -> Skip NPC Speech |
| Shrink/enlarge viewport | [ / ] | Actions -> named view command |
| Shrink/enlarge map panel | - / = | Actions -> named map command |
| Map zoom in/out | , / . | Actions -> named map command |
| Aircar guns narrow/widen/center | Z / C / V | Actions -> named aircar command |

There is no separate reload command in the original game. R centers the view.
Holding right mouse/LT preserves the original hand-mode behavior, which stops
movement on foot. Close or release it to return to normal movement.

## Menus, selectors, saving and loading

- Arrows or WASD navigate; Enter/Space accepts; Esc cancels selectors or returns; Backspace clears
  selections where supported. Y/N answer confirmation questions.
- Mouse pointer + left/right clicks retain original menu and shop behavior.
  The wheel navigates menu/selector items. With a controller use the left
  stick/D-pad, A/B/X, right-stick pointer, RT/LT clicks and RB/LB for Yes/No.
- In Options: **Q/E** or **LT/RT** cycle Game/Display/Actions tabs. Click a tab
  to select it. **Page Up/Page Down** or **LB/RB** change Actions subpages.
  Enter/A or a pointer click selects a native-page setting/action. Esc/B backs out.
- Save/load through the original Esc/Start menu, then select a slot. Save names
  use the local `YYYY-MM-DD HH:MM:SS` timestamp automatically; no typing needed.
- F1 opens the keyboard/mouse guide. Actions chooses the last-used input guide.
  Left/Right or LB/RB switches between guides; Esc/B closes. Guides pause the game.
- Esc, Enter, Space or A/B/Start skip movies. Input is cleared on movie return.

Held inputs are suppressed at host-overlay dismissal; context changes suppress
held keyboard movement and keep mouse buttons in their original context, so
held fire does not become a menu click and hand aiming remains held.
Original menu artwork/font is loaded from your own commercial game assets;
the two illustrated guide pages are generated artwork embedded in the engine.
