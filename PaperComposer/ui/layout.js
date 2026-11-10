// Frame behaviour that is about layout only (styles.css does the rest):
//  - narrow windows (≤1100px): the navigator and the right panel are drawers,
//    opened from the two panel buttons in the top bar (SlideComposer's way);
//    Esc or a click on the page closes them, picking a heading closes the navigator;
//  - a folded block's whole head opens it (寫作 starts folded).
(() => {
  ICONS.panelLeft = '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>';
  ICONS.panelRight = '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>';
  const app = document.querySelector(".app");
  const narrow = matchMedia("(max-width: 1100px)");
  const buttons = { left: document.getElementById("toggleLeft"), right: document.getElementById("toggleRight") };
  buttons.left.append(icon("panelLeft"));
  buttons.right.append(icon("panelRight"));

  function show(which, on) {
    const cls = `show-${which}`;
    on ??= !app.classList.contains(cls);
    app.classList.toggle(cls, on);
    buttons[which].setAttribute("aria-pressed", String(on));
  }
  const closeAll = () => { show("left", false); show("right", false); };
  buttons.left.onclick = () => show("left");
  buttons.right.onclick = () => show("right");
  narrow.addEventListener("change", closeAll);

  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !narrow.matches || document.querySelector("dialog[open]")) return;
    if (app.classList.contains("show-left") || app.classList.contains("show-right")) closeAll();
  });
  // a click on the page closes the drawers (menus and popovers keep them)
  document.getElementById("pageArea").addEventListener("pointerdown", () => { if (narrow.matches) closeAll(); });
  // jumping to a paragraph from the navigator: show the page
  document.querySelector("aside.left").addEventListener("click", (e) => {
    if (narrow.matches && e.target.closest(".outline button[data-s][data-p]")) show("left", false);
  });

  // folded block: a click anywhere on its head (not its ? help) opens it through its own toggle
  document.querySelector("aside.right").addEventListener("click", (e) => {
    const head = e.target.closest(".block[data-open='false'] > .block-head");
    if (!head || e.target.closest(".help, .toggle")) return;
    head.querySelector(".toggle")?.click();
  });
})();
