'use strict';
const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('probe', {
  mainChecks: () => ipcRenderer.invoke('main-checks'),
  copy: (text) => ipcRenderer.invoke('copy', text),
});
