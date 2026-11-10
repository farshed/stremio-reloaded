(function () {
  if (window.qt && window.qt.webChannelTransport) return;

  var MSG_SIGNAL = 1;
  var MSG_INIT = 3;
  var MSG_INVOKE_METHOD = 6;
  var MSG_RESPONSE = 10;
  var OBJECT_NAME = "transport";
  var EVENT_SIGNAL = 5;
  var ON_EVENT_METHOD = 6;

  function invoke(cmd, args) {
    return window.__TAURI_INTERNALS__.invoke(cmd, args);
  }

  function deliver(payload) {
    if (typeof transport.onmessage === "function") {
      transport.onmessage({ data: JSON.stringify(payload) });
    }
  }

  function describe(props) {
    return {
      methods: [["onEvent", ON_EVENT_METHOD]],
      signals: [["event", EVENT_SIGNAL]],
      properties: [
        [0, "objectName", [1, 0], OBJECT_NAME],
        [1, "shellVersion", [1, 1], props.shellVersion],
        [2, "serverAddress", [1, 2], props.serverAddress],
        [3, "isFullscreen", [1, 3], props.isFullscreen],
      ],
      enums: {},
    };
  }

  var transport = {
    onmessage: null,
    send: function (raw) {
      var msg = JSON.parse(raw);
      if (msg.type === MSG_INIT) {
        invoke("shell_props").then(
          function (props) {
            deliver({ type: MSG_RESPONSE, id: msg.id, data: { transport: describe(props) } });
          },
          function (err) {
            console.error("[shell] could not read shell properties", err);
          }
        );
      } else if (msg.type === MSG_INVOKE_METHOD && msg.object === OBJECT_NAME && msg.method === ON_EVENT_METHOD) {
        var ev = msg.args[0];
        var args = msg.args[1] === undefined ? null : msg.args[1];
        invoke("shell_event", { ev: ev, args: args }).catch(function (err) {
          console.error("[shell] event failed: " + ev, err);
        });
      }
    },
  };

  window.qt = { webChannelTransport: transport };

  Object.defineProperty(window, "__stremioShellEmit", {
    value: function (ev, args) {
      deliver({ type: MSG_SIGNAL, object: OBJECT_NAME, signal: EVENT_SIGNAL, args: [ev, args] });
    },
  });

  window.addEventListener("load", function () {
    if (typeof window.initShellComm !== "function") return;
    try {
      window.initShellComm();
    } catch (err) {
      console.error("[shell] initShellComm failed", err);
    }
  });
})();
