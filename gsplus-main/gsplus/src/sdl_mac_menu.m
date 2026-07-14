/**********************************************************************/
/*                    GSplus - Apple //gs Emulator                    */
/*                    Based on KEGS by Kent Dickey                    */
/*      This code is covered by the GNU GPL v3                        */
/**********************************************************************/

/* macOS-only helper for the SDL3 build.
 *
 * SDL3 installs a default macOS menu bar (Apple / App / Window menus) whose
 * items carry the usual Command-key equivalents: ⌘Q quits, ⌘W closes the
 * window, ⌘H hides, ⌘M minimizes. AppKit's responder chain consumes those
 * combos before SDL ever sees the keyDown, so the emulated IIgs -- which uses
 * ⌘ as the Open-Apple key and expects to receive ⌘Q, ⌘W, etc. -- never gets
 * them. That breaks any IIgs software bound to those combos.
 *
 * sdl_mac_fix_menu() strips the key equivalent from every menu item so all ⌘
 * combos fall through to SDL (and on to the emulated ADB), then rebinds the
 * Quit item alone to ⌥⌘Q (Option+Command+Q) -- a combo no IIgs software uses
 * -- so there's still a keyboard way to quit GSplus itself.
 */

#import <Cocoa/Cocoa.h>

/* Declared in protos_sdl.h for the C callers; that header pulls in defc.h
 * types we don't want here, so we just restate the one prototype. */
void	sdl_mac_fix_menu(void);

static void
gsplus_strip_key_equivalents(NSMenu *menu)
{
	for (NSMenuItem *item in menu.itemArray) {
		if (item.action == @selector(terminate:)) {
			/* Quit GSplus -> ⌥⌘Q, off-limits to IIgs software. */
			item.keyEquivalent = @"q";
			item.keyEquivalentModifierMask =
				NSEventModifierFlagCommand |
				NSEventModifierFlagOption;
		} else {
			/* Drop the shortcut so the ⌘ combo reaches the IIgs.
			 * The item stays in the menu and is still clickable. */
			item.keyEquivalent = @"";
			item.keyEquivalentModifierMask = 0;
		}
		if (item.submenu) {
			gsplus_strip_key_equivalents(item.submenu);
		}
	}
}

void
sdl_mac_fix_menu(void)
{
	@autoreleasepool {
		NSMenu *menu = [NSApp mainMenu];
		if (menu) {
			gsplus_strip_key_equivalents(menu);
		}
	}
}
