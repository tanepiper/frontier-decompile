# Shaders

From the early 2000s, much of what a 3D game looks like is decided by shaders: small programs
that run on the graphics card for each vertex and each pixel. Lighting, fog, water, the
blending of terrain textures and the colour grading of the whole screen are all in them. A port
that draws the same frames must copy them. Older games set up the fixed-function pipeline
instead, and a port must copy those settings the same way ([3D graphics](graphics-3d.md)). This
page covers finding the shaders, reading them, and porting them.

## Where they are

![The forms shaders shipped in, by year](../assets/diagrams/shader-forms.svg)

| Form | Era and API | How to find it |
|---|---|---|
| Fixed-function state | Direct3D 7 and earlier, OpenGL 1.x | Calls that set texture stages, blend modes, fog and lights. Log them with a frame capture tool or an API trace |
| ARB and NV assembly | OpenGL, about 2002 to 2006 | Text starting with `!!ARBvp1.0`, `!!ARBfp1.0`, `!!VP1.0` |
| Direct3D 8 and 9 bytecode | Shader models 1.1 to 3.0, about 2001 to 2012 | A version token (`0xFFFE0200` is `vs_2_0`, `0xFFFF0300` is `ps_3_0`) and an end token `0x0000FFFF`. Compiled shaders usually carry a constant table (CTAB) with the names of their inputs |
| Effect files | Direct3D 9 and 10 | `.fx` source or compiled `.fxo`, with techniques and passes that also set the render state |
| DXBC | Direct3D 10 and 11 | A container starting with `DXBC`, of chunks: RDEF (constants), ISGN and OSGN (inputs and outputs), SHDR or SHEX (code) |
| DXIL | Direct3D 12 | The same container with a DXIL chunk of LLVM bitcode |
| SPIR-V | Vulkan, later OpenGL | Words starting with `0x07230203` |
| GLSL and HLSL source | Any OpenGL game, and engines that compile when they load | Text: `#version`, `void main(`, `gl_FragColor`, `: SV_Target` |

Some engines also keep shader source beside the compiled code. Banished's materials carry GLSL
3.3 source generated from the engine's own shading language, next to the Direct3D 9 and 11
bytecode, although the shipped game has no OpenGL renderer. Such source is the clearest reference
for a port. Check that it matches the compiled shader the game uses before relying on it.

`tools/formats/shaders.py FILE` finds all of these in an executable, an archive or a data file,
prints each one's kind, version, size and names, and with `--dump DIR` writes each to its own
file. Run it on the files that `filetypes.py` could not identify. Packed archives must be
unpacked first.

## Reading them

- **Disassemble.** Microsoft's shader compiler (`fxc /dumpbin`) disassembles Direct3D 9 to 11
  shaders. `dxc -dumpbin` reads DXIL. `spirv-dis` reads SPIR-V, and `spirv-cross` turns it into
  GLSL or HLSL.
- **Decompile.** Community decompilers turn DXBC back into HLSL-like code. 3Dmigoto, made for
  stereoscopic and modding fixes, dumps and replaces the shaders of a running Direct3D 11 game.
- **Capture a frame.** RenderDoc (Direct3D 11 and 12, OpenGL, Vulkan) and PIX (Direct3D) record
  one frame: every draw call, its shaders, the constants they received, and the textures and
  buffers bound. This tells you which shader draws which object, and with what inputs. Direct3D
  9 games can be captured through a layer that translates them to a newer API, such as DXVK.
- **Name the constants.** The constant table gives names such as `FogColor` and `WorldViewProj`.
  The code that sets them (`SetVertexShaderConstantF` and its later forms) shows where each value
  comes from.

## Porting them

Port each shader into the port's shading language (WGSL, GLSL, HLSL or MSL) by hand,
instruction by instruction from the disassembly, keeping the original's order of operations.
Points that change the picture:

- **Precision.** Shader model 2 pixel shaders on some cards worked at 24-bit precision, and
  older cards at lower ones. A port at 32-bit gives slightly different results. This is usually
  acceptable and should be recorded as a deviation when it is visible.
- **Gamma.** Old games blend colours as stored, without converting from sRGB. A port that uses
  sRGB textures and render targets blends differently. Keep the original's choice.
- **Fixed-function emulation.** Fog, alpha test and texture stages must be written as shader
  code in a modern API. Copy the formulas exactly: linear or exponential fog, by depth or by
  distance, per vertex or per pixel.
- **Half-texel offsets.** Direct3D 9 placed pixel centres half a texel away from where later APIs
  put them. Full-screen effects and text are blurred or shifted by half a pixel if this is not
  corrected.
- **Texture sampling.** Filtering, mip-map bias, clamping and wrapping, and the original's
  texture formats (DXT compression decodes slightly differently on different cards).

Compare the port's frames with the original's, at the same camera and time, pixel by pixel
([Drawing](../observe/drawing.md)). Differences cluster where a shader differs.
