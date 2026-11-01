// @ts-check
import starlight from "@astrojs/starlight";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";
import starlightAutoSidebar from "starlight-auto-sidebar";
import starlightImageZoom from "starlight-image-zoom";
import starlightLlmsTxt from "starlight-llms-txt";

export default defineConfig({
  site: "https://paron.bovolato.dev",
  vite: { plugins: [tailwindcss()] },
  integrations: [
    starlight({
      title: "Paron",
      description:
        "Paron is a self-hosted SSH gateway in one binary. It reaches machines directly or through an outbound tunnel, keeps the credentials, and gives each user a terminal in the browser.",
      favicon: "/favicon.svg",
      customCss: ["./src/styles/site.css"],
      components: {
        Footer: "./src/components/Footer.astro",
        Header: "./src/components/Header.astro",
        Hero: "./src/components/Hero.astro",
        ThemeSelect: "./src/components/ThemeSelect.astro",
      },
      plugins: [
        starlightAutoSidebar(),
        starlightImageZoom({ showCaptions: false }),
        starlightLlmsTxt(),
      ],
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/nicolabovolato/paron",
        },
      ],
      editLink: {
        baseUrl:
          "https://github.com/nicolabovolato/paron/edit/master/packages/website/",
      },
      sidebar: [{ autogenerate: { directory: "docs" } }],
    }),
  ],
});
