import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: process.env.VITE_BASE_PATH ?? "/",
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ["VITE_"],
  build: { outDir: "dist", sourcemap: true }
});
