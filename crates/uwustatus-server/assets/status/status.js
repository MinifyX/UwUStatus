// Optional: arrow keys move between day bars, Escape hides the tooltip. The page works without it.
document.querySelectorAll("ol.bars").forEach(function (list) {
  var days = Array.prototype.slice.call(list.children);
  list.addEventListener("keydown", function (e) {
    var i = days.indexOf(document.activeElement);
    if (e.key === "Escape") { list.classList.add("hush"); return; }
    var step = { ArrowLeft: -1, ArrowRight: 1, Home: -i, End: days.length - 1 - i }[e.key];
    if (i < 0 || step === undefined) return;
    e.preventDefault();
    var next = days[Math.max(0, Math.min(days.length - 1, i + step))];
    if (next.offsetParent === null) return;
    days[i].tabIndex = -1;
    next.tabIndex = 0;
    next.focus();
    list.classList.remove("hush");
  });
  list.addEventListener("pointerover", function () { list.classList.remove("hush"); });
});
