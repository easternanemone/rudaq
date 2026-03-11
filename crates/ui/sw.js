// Minimal service worker for PWA installability.
// Caches the app shell so the install prompt appears on iPad/Android.
// Does NOT cache gRPC-web API calls — those always go to the network.

const CACHE_NAME = 'daq-panel-v1';

self.addEventListener('install', (event) => {
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((names) =>
      Promise.all(names.filter((n) => n !== CACHE_NAME).map((n) => caches.delete(n)))
    )
  );
  self.clients.claim();
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);

  // Never cache gRPC-web or API requests
  if (event.request.method !== 'GET' ||
      url.pathname.startsWith('/protocol.') ||
      event.request.headers.get('content-type')?.includes('grpc')) {
    return;
  }

  // Network-first for app shell assets (WASM, JS, HTML)
  event.respondWith(
    caches.open(CACHE_NAME).then((cache) =>
      fetch(event.request)
        .then((response) => {
          if (response.ok) {
            cache.put(event.request, response.clone());
          }
          return response;
        })
        .catch(() => cache.match(event.request))
    )
  );
});
