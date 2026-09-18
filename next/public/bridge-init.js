(() => {
  let tries = 0;

  const init = () => {
    if (window.throwback) return;
    if (!window.qt || !window.qt.webChannelTransport) {
      tries += 1;
      setTimeout(init, tries < 200 ? 50 : 1000);
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
