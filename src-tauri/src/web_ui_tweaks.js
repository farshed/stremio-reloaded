(function () {
  if (window.top !== window) return;

  var TYPE_SELECTS = '#discover-filters select[name="type"], #libraryFilters select[name="selectedType"]';
  var STYLES =
    ".shell-type-select{display:none !important}" +
    ".shell-type-toggle{flex:0 0 auto;margin-right:1rem;padding-left:0 !important}" +
    ".shell-type-toggle li:last-child{margin-right:0}";

  function getInjector() {
    var ng = window.angular;
    var injector = ng && document.body && ng.element(document.body).injector();
    return injector && injector.has("libraryItem") ? injector : null;
  }

  function treatReaddedItemsAsNew(injector) {
    var library = injector.get("libraryItem");
    var addToLibrary = library.addToLibrary;
    library.addToLibrary = function (item) {
      var existing = item && library.byIdAll[item._id];
      if (existing && existing.removed) {
        var now = new Date();
        existing.state.lastWatched = now;
        existing._ctime = now;
      }
      return addToLibrary.apply(this, arguments);
    };
  }

  function typeLabel(select, option) {
    var scope = window.angular.element(select).scope();
    if (scope && scope.translate && scope.plural) return scope.translate(scope.plural(option.value));
    return option.text;
  }

  function renderTypeToggle(toggle, select, container) {
    var options = Array.prototype.filter.call(select.options, function (option) {
      return option.value && option.value.charAt(0) !== "?";
    });
    var hidden = container.classList.contains("ng-hide");
    var signature =
      options.map(function (o) { return o.value + "=" + typeLabel(select, o); }).join(",") +
      "|" + select.value + "|" + hidden;
    if (toggle.dataset.signature === signature) return;
    toggle.dataset.signature = signature;

    toggle.style.display = hidden ? "none" : "";
    toggle.textContent = "";
    options.forEach(function (option) {
      var item = document.createElement("li");
      item.tabIndex = -1;
      item.dataset.value = option.value;
      item.textContent = typeLabel(select, option);
      if (option.value === select.value) item.classList.add("selected");
      toggle.appendChild(item);
    });
  }

  function syncTypeToggles() {
    document.querySelectorAll(TYPE_SELECTS).forEach(function (select) {
      var container = select.closest(".custom-select");
      if (!container) return;
      var toggle = container.nextElementSibling;
      if (!toggle || !toggle.classList.contains("shell-type-toggle")) {
        toggle = document.createElement("ul");
        toggle.className = "segments shell-type-toggle";
        toggle.addEventListener("click", function (event) {
          var item = event.target.closest("li[data-value]");
          if (!item || select.value === item.dataset.value) return;
          select.value = item.dataset.value;
          select.dispatchEvent(new Event("change", { bubbles: true }));
        });
        container.parentNode.insertBefore(toggle, container.nextSibling);
        container.classList.add("shell-type-select");
      }
      renderTypeToggle(toggle, select, container);
    });
  }

  function useTypeToggles(injector) {
    var style = document.createElement("style");
    style.textContent = STYLES;
    document.head.appendChild(style);

    var scheduled = false;
    injector.get("$rootScope").$watch(function () {
      if (scheduled) return;
      scheduled = true;
      setTimeout(function () {
        scheduled = false;
        syncTypeToggles();
      }, 0);
    });
  }

  var tweaks = [treatReaddedItemsAsNew, useTypeToggles];

  window.addEventListener("load", function () {
    var attempts = 0;
    (function attempt() {
      var injector = getInjector();
      if (!injector) {
        if (++attempts < 50) setTimeout(attempt, 200);
        return;
      }
      tweaks.forEach(function (tweak) {
        try {
          tweak(injector);
        } catch (err) {
          console.error("[shell] web UI tweak failed: " + tweak.name, err);
        }
      });
    })();
  });
})();
