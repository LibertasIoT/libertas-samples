// tsdown.config.ts
// requires npm install --save-dev tsdown
import { defineConfig } from 'tsdown';

export default defineConfig({
    entry: ['src/index.ts'],
    format: ['esm', 'cjs'], // Outputs both modern ESM and fallback CommonJS targets
    dts: true,              // ◄── Automatically bundles your type definition files (.d.ts)
    clean: true,            // Wipes the dist/ folder before rebuilding
    minify: true
});
