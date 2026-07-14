/**********************************************************************/
/*                    GSplus - Apple //gs Emulator                    */
/*                    Based on KEGS by Kent Dickey                    */
/*      This code is covered by the GNU GPL v3                        */
/**********************************************************************/

/* Public prototypes for the SDL3 driver (sdl_driver.c / sdl_snd_driver.c).
 * Relies on byte/word32 from defc.h, so include defc.h before this header. */

#ifndef GSPLUS_PROTOS_SDL_H
#define GSPLUS_PROTOS_SDL_H

/* sdl_snd_driver.c */
void	sdl_snd_init(word32 *shmaddr);
int	sdl_send_audio(byte *ptr, int size);
void	sdl_snd_shutdown(void);

/* png_write.c */
int	write_png_rgba(const char *path, const unsigned char *rgba,
			int w, int h);

#ifdef __APPLE__
/* sdl_mac_menu.m -- retune SDL's default macOS menu so ⌘ combos reach the
 * IIgs and GSplus quits on ⌥⌘Q instead of ⌘Q. */
void	sdl_mac_fix_menu(void);
#endif

#endif /* GSPLUS_PROTOS_SDL_H */
