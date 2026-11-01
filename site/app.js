"use strict";
const previews = {
  capture: {
    title: "区域截图 · 原位进入标注",
    alt: "实际截图界面：框选区域和紧贴选区的标注工具栏",
  },
  pin: {
    title: "独立贴图 · 资料留在手边",
    alt: "实际贴图窗口与边框、阴影、另存为右键菜单",
  },
  annotation: {
    title: "图片标注 · 让重点一目了然",
    alt: "实际标注界面：在原图上绘制矩形和箭头",
  },
  "long-capture": {
    title: "长截图 · 连续内容完整拼接",
    alt: "实际拼接结果：四段连续内容组成的长截图",
  },
  gif: {
    title: "GIF 录制 · 记录完整过程",
    alt: "实际 GIF 录制界面：区域边框与录制控制栏",
  },
  trail: {
    title: "鼠标拖尾 · 动画与配色随心选",
    alt: "实际鼠标拖尾设置：四种动画、配色选择和效果预览",
  },
};
const screenshot = (name) => `assets/screenshots/${name}.jpg`;
document.querySelectorAll("[data-demo]").forEach((button) => {
  button.addEventListener("click", () => {
    const name = button.dataset.demo;
    const preview = previews[name];
    document
      .querySelectorAll("[data-demo]")
      .forEach((tab) =>
        tab.setAttribute("aria-pressed", String(tab === button)),
      );
    const image = document.getElementById("showcase-image");
    image.src = screenshot(name);
    image.alt = preview.alt;
    document.getElementById("showcase-label").textContent = preview.title;
    document.getElementById("demo-note").textContent =
      `${preview.title} · 实际应用界面`;
  });
});
const dialog = document.querySelector(".image-dialog");
let returnFocus;
document.querySelectorAll("[data-image]").forEach((button) => {
  button.addEventListener("click", () => {
    const name = button.dataset.image;
    const image = document.getElementById("dialog-image");
    image.src = screenshot(name);
    image.alt = previews[name].alt;
    document.getElementById("image-caption").textContent = previews[name].title;
    returnFocus = button;
    dialog.showModal();
  });
});
document
  .querySelector(".dialog-close")
  .addEventListener("click", () => dialog.close());
dialog.addEventListener("click", (event) => {
  const bounds = dialog.getBoundingClientRect();
  if (
    event.clientX < bounds.left ||
    event.clientX > bounds.right ||
    event.clientY < bounds.top ||
    event.clientY > bounds.bottom
  )
    dialog.close();
});
dialog.addEventListener("close", () =>
  returnFocus?.focus({ preventScroll: true }),
);

// Release links remain usable when GitHub's public API is offline or rate-limited.
fetch("https://api.github.com/repos/junyi-deep/simple-snip/releases/latest", {
  signal: AbortSignal.timeout(4000),
})
  .then((response) => {
    if (!response.ok) throw new Error("Release API unavailable");
    return response.json();
  })
  .then((release) => {
    if (!Array.isArray(release.assets)) return;
    for (const [platform, ending] of [
      ["windows", "_win-amd64_unsigned.zip"],
      ["macos", "_macos-arm64_unsigned.dmg"],
    ]) {
      const asset = release.assets.find((asset) =>
        asset.name?.endsWith(ending),
      );
      if (
        asset?.browser_download_url?.startsWith(
          "https://github.com/junyi-deep/simple-snip/releases/download/",
        )
      ) {
        document.querySelector(`[data-platform="${platform}"]`).href =
          asset.browser_download_url;
      }
    }
  })
  .catch(() => {});
