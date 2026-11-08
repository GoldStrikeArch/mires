import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'

const contentTypes = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
}

export async function startServer(routes) {
  const server = createServer(async (request, response) => {
    try {
      const pathname = new URL(request.url, 'http://127.0.0.1').pathname
      const route = routes[pathname]
      if (!route) {
        response.writeHead(404).end('Not found')
        return
      }
      const body = route.file ? await readFile(route.file) : route.body
      const extension = route.type || pathname.slice(pathname.lastIndexOf('.'))
      response.writeHead(200, {
        'cache-control': 'no-store',
        'content-type': contentTypes[extension] || 'application/octet-stream',
      }).end(body)
    } catch (error) {
      response.writeHead(500).end(String(error))
    }
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const { port } = server.address()
  return {
    origin: `http://127.0.0.1:${port}`,
    close: () => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())),
  }
}
