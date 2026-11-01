(function () {
  var theme = null;

  try {
    theme = localStorage.getItem("paron-theme");
  } catch (error) {}

  if (theme !== "light" && theme !== "dark") {
    theme = matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }

  document.documentElement.classList.add(theme);
})();
