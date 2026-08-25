// Getting the console's framebuffer onto the page.
//
// The PPU hands us RGBA8888 sitting in wasm memory. WebGL can upload that
// straight to a texture, so the pixels go from the guest's VRAM to the screen
// without JavaScript ever copying them — `putImageData` needs an intermediate
// `ImageData` buffer, WebGL does not.
//
// At 240x160 that saving is far too small to matter: the whole blit costs about
// a hundredth of a millisecond either way. What the shader buys is control of
// the upscale, and somewhere to put a post-process later if one is wanted.
//
// If WebGL is unavailable, `createRenderer` falls back to canvas 2D and the
// picture is identical.

const VERTEX = `#version 300 es
// A single triangle big enough to cover the viewport. No vertex buffer needed:
// the positions come from gl_VertexID.
const vec2 CORNERS[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
out vec2 uv;
void main() {
  vec2 p = CORNERS[gl_VertexID];
  // Flip vertically: texture rows start at the top, clip space at the bottom.
  uv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  gl_Position = vec4(p, 0.0, 1.0);
}`;

const PASSTHROUGH = `#version 300 es
precision highp float;
uniform sampler2D frame;
in vec2 uv;
out vec4 colour;
void main() { colour = texture(frame, uv); }`;

function compile(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`shader failed to compile: ${log}`);
  }
  return shader;
}

function link(gl, fragment) {
  const program = gl.createProgram();
  const vs = compile(gl, gl.VERTEX_SHADER, VERTEX);
  const fs = compile(gl, gl.FRAGMENT_SHADER, fragment);
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`program failed to link: ${log}`);
  }
  return program;
}

function glRenderer(canvas, width, height) {
  const gl = canvas.getContext("webgl2", {
    alpha: false,
    antialias: false,
    depth: false,
    stencil: false,
    // So the finished frame can still be read back — the end-to-end test
    // inspects what actually reached the screen.
    preserveDrawingBuffer: true,
  });
  if (!gl) return null;

  let ready = false;

  const setup = () => {
    const program = link(gl, PASSTHROUGH);
    gl.useProgram(program);

    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    // NEAREST is the whole point: this is a 240x160 display, and the upscale
    // must not blur it.
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);

    gl.uniform1i(gl.getUniformLocation(program, "frame"), 0);

    // WebGL2 requires a bound vertex array even when drawing from gl_VertexID.
    gl.bindVertexArray(gl.createVertexArray());
    gl.viewport(0, 0, canvas.width, canvas.height);
    ready = true;
  };

  // A lost context leaves every object above invalid; rebuild them on restore.
  canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    ready = false;
  });
  canvas.addEventListener("webglcontextrestored", setup);
  setup();

  return {
    kind: "webgl2",
    resize() {
      gl.viewport(0, 0, canvas.width, canvas.height);
    },
    draw(pixels) {
      if (!ready) return;
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
  };
}

function canvas2dRenderer(canvas, width, height) {
  // The drawing buffer matches the console, and CSS does the magnification.
  const displayWidth = canvas.style.width;
  const displayHeight = canvas.style.height;
  canvas.width = width;
  canvas.height = height;
  canvas.style.width = displayWidth;
  canvas.style.height = displayHeight;
  const context = canvas.getContext("2d");
  const image = context.createImageData(width, height);
  return {
    kind: "canvas2d",
    resize() {},
    draw(pixels) {
      image.data.set(pixels);
      context.putImageData(image, 0, 0);
    },
  };
}

/**
 * Build a renderer for a `width` x `height` console screen, magnified `scale`
 * times. Returns an object with `draw(pixels)` and a `kind` naming the backend.
 */
export function createRenderer(canvas, width, height, { scale = 3 } = {}) {
  // The GPU path magnifies for us, so the drawing buffer is the display size.
  const size = (factor) => {
    canvas.width = width * factor;
    canvas.height = height * factor;
    canvas.style.width = `${width * factor}px`;
    canvas.style.height = `${height * factor}px`;
  };
  size(scale);

  let renderer;
  try {
    renderer = glRenderer(canvas, width, height);
  } catch (error) {
    console.warn("WebGL unavailable, falling back to canvas 2D:", error);
  }
  renderer = renderer ?? canvas2dRenderer(canvas, width, height);

  return {
    ...renderer,
    /// Re-magnify, for a window that changed shape or went full screen.
    setScale(factor) {
      size(factor);
      renderer.resize();
    },
  };
}
