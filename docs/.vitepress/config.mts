import { defineConfig } from "vitepress";

export default defineConfig({
  lang: "zh-CN",
  title: "SubForge",
  description: "本地配置聚合平台的使用与部署文档",
  lastUpdated: true,
  cleanUrls: true,
  themeConfig: {
    logo: "/logo.svg",
    nav: [
      { text: "首页", link: "/" },
      { text: "快速开始", link: "/quick-start" },
      { text: "配置", link: "/guide/configuration" },
      { text: "API", link: "/api/overview" },
      { text: "插件", link: "/plugins/overview" },
      { text: "部署", link: "/deploy/headless" }
    ],
    sidebar: [
      {
        text: "入门",
        items: [
          { text: "快速开始", link: "/quick-start" },
          { text: "配置文件", link: "/guide/configuration" },
          { text: "架构总览", link: "/guide/architecture" },
          { text: "安全模型", link: "/guide/security" }
        ]
      },
      {
        text: "插件",
        items: [
          { text: "插件体系", link: "/plugins/overview" },
          { text: "静态插件", link: "/plugins/static" },
          { text: "脚本开发", link: "/plugins/script" }
        ]
      },
      {
        text: "接口",
        items: [
          { text: "HTTP API 总览", link: "/api/overview" }
        ]
      },
      {
        text: "部署",
        items: [
          { text: "无头部署", link: "/deploy/headless" },
          { text: "常见问题", link: "/faq" }
        ]
      }
    ],
    socialLinks: [
      { icon: "github", link: "https://github.com/SubFoundry/SubForge" }
    ],
    footer: {
      message: "SubForge Documentation",
      copyright: "Copyright © 2026 SubForge"
    }
  }
});
