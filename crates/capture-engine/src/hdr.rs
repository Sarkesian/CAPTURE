//! HDR desktop composition is linear scRGB (Rec.709 primaries, 1.0 = 80 cd/m²).
//! Convert on the GPU to absolute PQ/Rec.2020 P010 for NVENC Main10, or explicitly
//! tone map to SDR for Compatible Recording. Never invent mastering/content-light metadata.
use crate::gpu::Gpu;
use windows::core::{Interface, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

pub const SHADER: &str = r#"
Texture2D<float4> source : register(t0);
float4 vs(uint id:SV_VertexID):SV_POSITION {
    float2 p=float2((id<<1)&2,id&2);return float4(p*float2(2,-2)+float2(-1,1),0,1);
}
float3 pq(float3 light) {
    float3 x=pow(saturate(light*80.0/10000.0),0.1593017578125);
    return pow((0.8359375+18.8515625*x)/(1.0+18.6875*x),78.84375);
}
float3 encoded(int2 pixel) {
    float3 rgb=source.Load(int3(pixel,0)).rgb;
    float3 bt2020=float3(dot(rgb,float3(.627403896,0.329283038,0.043313066)),
        dot(rgb,float3(.069097289,0.919540395,0.011362316)),
        dot(rgb,float3(.016391439,0.088013308,0.895595253)));
    return pq(bt2020);
}
float3 yuv(float3 rgb) {float y=dot(rgb,float3(.2627,.6780,.0593));return float3(y,(rgb.b-y)/1.8814,(rgb.r-y)/1.4746);}
float4 ps_luma(float4 pos:SV_POSITION):SV_TARGET {float y=yuv(encoded(int2(pos.xy))).x;return float4(round(64+876*y)*64/65535.0,0,0,1);}
float4 ps_chroma(float4 pos:SV_POSITION):SV_TARGET {
    int2 p=int2(pos.xy)*2;
    float2 uv=(yuv(encoded(p)).yz+yuv(encoded(p+int2(1,0))).yz+yuv(encoded(p+int2(0,1))).yz+yuv(encoded(p+int2(1,1))).yz)*.25;
    return float4(round(512+896*uv)*64/65535.0,0,1);
}
// Hue-preserving luminance shoulder with a fixed knee at 75% of Windows SDR white.
// Normalize SDR desktop brightness before mapping, never infer a game's mastering peak.
float4 ps_sdr(float4 pos:SV_POSITION):SV_TARGET {
    float3 rgb=max(source.Load(int3(int2(pos.xy),0)).rgb,0)*80.0/SDR_WHITE;
    float l=dot(rgb,float3(.2126,.7152,.0722));
    float mapped=l<=.75 ? l : .75+.25*(1-exp(-4*(l-.75)));
    rgb=saturate(rgb*(mapped/max(l,1e-8)));
    float3 encoded=lerp(12.92*rgb,1.055*pow(rgb,1.0/2.4)-.055,step(.0031308,rgb));
    return float4(encoded,1);
}
"#;

unsafe fn compile(entry: &[u8], target: &[u8]) -> Result<ID3DBlob, String> {
    compile_source(&SHADER.replace("SDR_WHITE", "203.0"), entry, target)
}
unsafe fn compile_source(source: &str, entry: &[u8], target: &[u8]) -> Result<ID3DBlob, String> {
    let (mut code, mut errors) = (None, None);
    D3DCompile(
        source.as_ptr().cast(),
        source.len(),
        PCSTR::null(),
        None,
        None,
        PCSTR(entry.as_ptr()),
        PCSTR(target.as_ptr()),
        0,
        0,
        &mut code,
        Some(&mut errors),
    )
    .map_err(|e| {
        errors
            .map(|b| {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    b.GetBufferPointer().cast(),
                    b.GetBufferSize(),
                ))
                .into_owned()
            })
            .unwrap_or(e.to_string())
    })?;
    Ok(code.unwrap())
}
unsafe fn blob(b: &ID3DBlob) -> &[u8] {
    std::slice::from_raw_parts(b.GetBufferPointer().cast(), b.GetBufferSize())
}

pub struct Converter {
    sdr: Option<ID3D11PixelShader>,
    vs: ID3D11VertexShader,
    y: ID3D11PixelShader,
    uv: ID3D11PixelShader,
    state: ID3DDeviceContextState,
    width: u32,
    height: u32,
}
unsafe impl Send for Converter {}
unsafe impl Sync for Converter {}
impl Converter {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Result<Self, String> {
        unsafe {
            if width % 2 != 0 || height % 2 != 0 {
            return Err("HDR and Compatible 4:2:0 recording require even native dimensions; no crop or resize was applied".into());
            }
            let (vs, y, uv) = (
                compile(b"vs\0", b"vs_5_0\0")?,
                compile(b"ps_luma\0", b"ps_5_0\0")?,
                compile(b"ps_chroma\0", b"ps_5_0\0")?,
            );
            let (mut v, mut py, mut pu, mut state) = (None, None, None, None);
            gpu.device
                .CreateVertexShader(blob(&vs), None, Some(&mut v))
                .map_err(|e| e.to_string())?;
            gpu.device
                .CreatePixelShader(blob(&y), None, Some(&mut py))
                .map_err(|e| e.to_string())?;
            gpu.device
                .CreatePixelShader(blob(&uv), None, Some(&mut pu))
                .map_err(|e| e.to_string())?;
            let dev: ID3D11Device1 = gpu.device.cast().map_err(|e| e.to_string())?;
            dev.CreateDeviceContextState(
                0,
                &[gpu.device.GetFeatureLevel()],
                D3D11_SDK_VERSION,
                &ID3D11Device::IID,
                None,
                Some(&mut state),
            )
            .map_err(|e| e.to_string())?;
            Ok(Self {
                sdr: None,
                vs: v.unwrap(),
                y: py.unwrap(),
                uv: pu.unwrap(),
                state: state.unwrap(),
                width,
                height,
            })
        }
    }
    pub fn new_sdr(gpu: &Gpu, width: u32, height: u32, white: f32) -> Result<Self, String> {
        if !white.is_finite() || white <= 0.0 {
            return Err("Invalid Windows SDR white level".into());
        }
        let mut c = Self::new(gpu, width, height)?;
        unsafe {
            let shader = compile_source(
                &SHADER.replace("SDR_WHITE", &format!("{white:.6}")),
                b"ps_sdr\0",
                b"ps_5_0\0",
            )?;
            gpu.device
                .CreatePixelShader(blob(&shader), None, Some(&mut c.sdr))
                .map_err(|e| e.to_string())?;
        }
        Ok(c)
    }
    pub fn convert(
        &self,
        gpu: &Gpu,
        source: &ID3D11Texture2D,
        dst: &ID3D11Texture2D,
        index: u32,
    ) -> Result<(), String> {
        unsafe {
            let dev: ID3D11Device3 = gpu
                .device
                .cast()
                .map_err(|e| format!("Planar HDR GPU views unavailable: {e}"))?;
            let ctx: ID3D11DeviceContext1 = gpu.context.cast().map_err(|e| e.to_string())?;
            let mut srv = None;
            gpu.device
                .CreateShaderResourceView(source, None, Some(&mut srv))
                .map_err(|e| e.to_string())?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            dst.GetDesc(&mut desc);
            let mut views = vec![];
            let planes = if self.sdr.is_some() { 1 } else { 2 };
            for plane in 0..planes {
                let rd = D3D11_RENDER_TARGET_VIEW_DESC1 {
                    Format: if self.sdr.is_some() {
                        DXGI_FORMAT_B8G8R8A8_UNORM
                    } else if plane == 0 {
                        DXGI_FORMAT_R16_UNORM
                    } else {
                        DXGI_FORMAT_R16G16_UNORM
                    },
                    ViewDimension: if desc.ArraySize > 1 {
                        D3D11_RTV_DIMENSION_TEXTURE2DARRAY
                    } else {
                        D3D11_RTV_DIMENSION_TEXTURE2D
                    },
                    Anonymous: if desc.ArraySize > 1 {
                        D3D11_RENDER_TARGET_VIEW_DESC1_0 {
                            Texture2DArray: D3D11_TEX2D_ARRAY_RTV1 {
                                MipSlice: 0,
                                FirstArraySlice: index,
                                ArraySize: 1,
                                PlaneSlice: plane,
                            },
                        }
                    } else {
                        D3D11_RENDER_TARGET_VIEW_DESC1_0 {
                            Texture2D: D3D11_TEX2D_RTV1 {
                                MipSlice: 0,
                                PlaneSlice: plane,
                            },
                        }
                    },
                };
                let mut rtv = None;
                dev.CreateRenderTargetView1(dst, Some(&rd), Some(&mut rtv))
                    .map_err(|e| format!("P010 plane {plane} rendering unavailable: {e}"))?;
                views.push(
                    rtv.unwrap()
                        .cast::<ID3D11RenderTargetView>()
                        .map_err(|e| e.to_string())?,
                );
            }
            let mt: ID3D11Multithread = gpu.context.cast().map_err(|e| e.to_string())?;
            mt.Enter();
            let mut old = None;
            ctx.SwapDeviceContextState(&self.state, Some(&mut old));
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShaderResources(0, Some(&[srv]));
            for plane in 0..planes as usize {
                ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    Width: (self.width >> plane) as f32,
                    Height: (self.height >> plane) as f32,
                    MaxDepth: 1.0,
                    ..Default::default()
                }]));
                ctx.PSSetShader(
                    self.sdr
                        .as_ref()
                        .unwrap_or(if plane == 0 { &self.y } else { &self.uv }),
                    None,
                );
                ctx.OMSetRenderTargets(Some(&[Some(views[plane].clone())]), None);
                ctx.Draw(3, 0);
            }
            ctx.PSSetShaderResources(0, Some(&[None]));
            ctx.OMSetRenderTargets(Some(&[None]), None);
            ctx.SwapDeviceContextState(old.as_ref(), None);
            mt.Leave();
            Ok(())
        }
    }
}

pub fn sdr_texture(gpu: &Gpu, width: u32, height: u32) -> Result<ID3D11Texture2D, String> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            ..Default::default()
        };
        let mut out = None;
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut out))
            .map_err(|e| e.to_string())?;
        Ok(out.unwrap())
    }
}

pub fn pq(nits: f64) -> f64 {
    let x = (nits / 10000.0).clamp(0.0, 1.0).powf(2610.0 / 16384.0);
    ((3424.0 / 4096.0 + 2413.0 / 128.0 * x) / (1.0 + 2392.0 / 128.0 * x)).powf(2523.0 / 32.0)
}
pub fn rgb2020(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    [
        0.627403896 * r + 0.329283038 * g + 0.043313066 * b,
        0.069097289 * r + 0.919540395 * g + 0.011362316 * b,
        0.016391439 * r + 0.088013308 * g + 0.895595253 * b,
    ]
}

pub fn half_to_float(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exp = (bits >> 10) & 31;
    let frac = (bits & 1023) as f32;
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp as i32 - 15),
    }
}

#[cfg(test)]
pub fn test_half(v: f32) -> u16 {
    if v == 0.0 {
        return 0;
    }
    let b = v.to_bits();
    let exp = ((b >> 23) & 255) as i32 - 127 + 15;
    (((b >> 16) & 0x8000) | ((exp.clamp(0, 30) as u32) << 10) | ((b >> 13) & 1023)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pq_and_primaries_reference() {
        assert!((pq(100.0) - 0.5080784215).abs() < 1e-8);
        assert!((pq(1000.0) - 0.7518270962).abs() < 1e-8);
        assert!((pq(10000.0) - 1.0).abs() < 1e-10);
        for v in rgb2020([1.0, 1.0, 1.0]) {
            assert!((v - 1.0).abs() < 1e-12);
        }
    }
    #[test]
    fn hdr_shader_compiles() {
        unsafe {
            compile(b"vs\0", b"vs_5_0\0").unwrap();
            compile(b"ps_luma\0", b"ps_5_0\0").unwrap();
            compile(b"ps_chroma\0", b"ps_5_0\0").unwrap();
        }
    }
}
