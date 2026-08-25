/** The three local inference engines, and what to call them on screen.
 *
 *  Its own module rather than part of `ipc.ts` because it is data, not a
 *  command: every test that mocks the IPC surface would otherwise blank the
 *  label map too, and a component reading `ENGINE_LABEL[engine]` would render
 *  `undefined` in the one place a human checks which engine is running.
 */

/** llama.cpp runs everywhere; MLX is Apple Silicon only and faster there;
 *  MTPLX reads the same MLX repositories and runs the MTP head they carry. */
export type LocalEngine = "llamacpp" | "mlx" | "mtplx";

/** Shared so the settings list and the status bar cannot drift apart — the
 *  status bar used to branch two ways over these three and read MTPLX, the
 *  fastest of them, as "llama.cpp". */
export const ENGINE_LABEL: Record<LocalEngine, string> = {
  llamacpp: "llama.cpp",
  mlx: "MLX",
  mtplx: "MTPLX",
};
