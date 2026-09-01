(() => {
  const FAST_TRIES = 200;
  const FAST_MS = 50;
  const SLOW_MS = 1000;
  let tries = 0;

  const init = () => {
    if (window.throwback) return;
    if (!window.qt || !window.qt.webChannelTransport) {
      tries += 1;
      setTimeout(init, tries < FAST_TRIES ? FAST_MS : SLOW_MS);
      return;
    }
    const script = document.createElement("script");
    script.src = "/qwebchannel.js";
    script.onload = () => {
      new QWebChannel(window.qt.webChannelTransport, (channel) => {
        window.throwback = channel.objects;
        window.dispatchEvent(new Event("throwback:ready"));
      });
    };
    document.head.appendChild(script);
  };

  init();
})();
