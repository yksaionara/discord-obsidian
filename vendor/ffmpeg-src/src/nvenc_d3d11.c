/* A Direct3D 11 device for NVENC's session on Windows, which
 * patches/nvenc-direct3d-device.patch has FFmpeg open in place of a CUDA
 * context. NVENC encodes the same from either; a CUDA context took 116 MiB
 * of the stream worker's memory (300 MiB committed) on an RTX 5070 Ti with
 * driver 32.0.16.2101, and a device beside the capture's own takes 4.
 *
 * FFmpeg's own Direct3D support (hwcontext_d3d11va.c) isn't built: it would
 * bring its hardware frames, where the encoders take frames from memory. */
#define COBJMACROS
#include <d3d11.h>
#include <dxgi.h>

#define NVIDIA 0x10de

/* A device on the `index`th NVIDIA graphics adapter, counted from 0, for the
 * caller to release; null when there's none or it gives no device. */
void *obsidian_nvenc_d3d11_device(int index) {
  IDXGIFactory1 *factory = NULL;
  if (FAILED(CreateDXGIFactory1(&IID_IDXGIFactory1, (void **)&factory))) return NULL;
  ID3D11Device *device = NULL;
  IDXGIAdapter1 *adapter = NULL;
  int found = 0;
  for (UINT at = 0; !found && IDXGIFactory1_EnumAdapters1(factory, at, &adapter) == S_OK; at++) {
    DXGI_ADAPTER_DESC1 desc;
    if (SUCCEEDED(IDXGIAdapter1_GetDesc1(adapter, &desc)) && desc.VendorId == NVIDIA && index-- == 0) {
      found = 1;
      if (FAILED(D3D11CreateDevice((IDXGIAdapter *)adapter, D3D_DRIVER_TYPE_UNKNOWN, NULL, 0, NULL, 0,
                                   D3D11_SDK_VERSION, &device, NULL, NULL)))
        device = NULL;
    }
    IDXGIAdapter1_Release(adapter);
  }
  IDXGIFactory1_Release(factory);
  return device;
}

void obsidian_nvenc_d3d11_release(void *device) {
  if (device) ID3D11Device_Release((ID3D11Device *)device);
}
