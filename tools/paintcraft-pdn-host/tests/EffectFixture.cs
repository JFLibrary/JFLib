// Real v5 effect plugins, independent of the host: what they compute comes from the paint.net API alone.
// Internal classes on purpose, the published samples are internal too.
using System.IO;
using System.Windows.Forms;
using PaintDotNet;
using PaintDotNet.AppModel;
using PaintDotNet.Clipboard;
using PaintDotNet.Direct2D1;
using PaintDotNet.Direct2D1.Effects;
using PaintDotNet.Effects;
using PaintDotNet.Effects.Gpu;
using PaintDotNet.Imaging;
using PaintDotNet.IndirectUI;
using PaintDotNet.PropertySystem;
using PaintDotNet.Rendering;
using IDeviceContext=PaintDotNet.Direct2D1.IDeviceContext;

internal sealed class BitmapInvert:PropertyBasedBitmapEffect
{
    IBitmapSource<ColorBgra32>? source;
    int mask;
    public BitmapInvert():base("Fixture Bitmap","Fixture",BitmapEffectOptions.Create() with {IsConfigurable=true}) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([new Int32Property("Mask",255,0,255),new Int32Property("Unlisted",1,0,9),
        new Int32Property("Tint",0xFF8000,0,0xFFFFFF),new DoubleVectorProperty("Point",Pair.Create(0.5,0.25),Pair.Create(-1.0,-1.0),Pair.Create(1.0,1.0))]);
    // Named here, and left out of the dialog like a plugin's internal value.
    protected override ControlInfo OnCreateConfigUI(PropertyCollection properties)
    {
        var ui=CreateDefaultConfigUI(properties.Where(p=>p.Name!="Unlisted"));
        ui.SetPropertyControlType("Tint",PropertyControlType.ColorWheel);
        ui.SetPropertyControlValue("Mask",ControlInfoPropertyNames.DisplayName,"Fixture-Maske");
        ui.SetPropertyControlValue("Mask",ControlInfoPropertyNames.Description,"Die umgekehrten Bits");
        return ui;
    }
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info) {source=Environment.GetSourceBitmapBgra32();base.OnInitializeRenderInfo(info);}
    protected override void OnSetToken(PropertyBasedEffectConfigToken? token) {mask=token!.GetProperty<Int32Property>("Mask")!.Value;base.OnSetToken(token);}
    protected override void OnRender(IBitmapEffectOutput output)
    {
        using var target=output.LockBgra32();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=ColorBgra32.FromBgra((byte)(c.B^mask),(byte)(c.G^mask),(byte)(c.R^mask),c.A);
        }
    }
}

// Asks for a floating-point output, which the renderer has to convert for its host.
internal sealed class BitmapFloat:PropertyBasedBitmapEffect
{
    IBitmapSource<ColorPrgba128Float>? source;
    public BitmapFloat():base("Fixture Bitmap Float","Fixture",BitmapEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info)
    {
        info.OutputPixelFormat=PixelFormats.Prgba128Float;
        source=Environment.GetSourceBitmap<ColorPrgba128Float>();
        base.OnInitializeRenderInfo(info);
    }
    protected override void OnRender(IBitmapEffectOutput output)
    {
        using var target=output.Lock<ColorPrgba128Float>();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=new ColorPrgba128Float(c.A-c.R,c.A-c.G,c.A-c.B,c.A);
        }
    }
}

// No token and no dialog, one thread, horizontal strips.
internal sealed class BitmapPlain:BitmapEffect
{
    IBitmapSource<ColorBgra32>? source;
    public BitmapPlain():base("Fixture Bitmap Plain","Fixture",BitmapEffectOptions.Create()) {}
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info)
    {
        info.Schedule=BitmapEffectRenderingSchedule.HorizontalStrips;
        info.Flags|=BitmapEffectRenderingFlags.SingleThreaded;
        source=Environment.GetSourceBitmapBgra32();
    }
    protected override void OnRender(IBitmapEffectOutput output)
    {
        using var target=output.LockBgra32();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=ColorBgra32.FromBgra((byte)(255-c.B),(byte)(255-c.G),(byte)(255-c.R),c.A);
        }
    }
}

// A classic effect without a dialog: asking it for one fails.
public sealed class ClassicPlain:Effect
{
    public ClassicPlain():base("Fixture Classic Plain",(System.Drawing.Image)null!,"Fixture",new EffectOptions()) {}
    public override void Render(EffectConfigToken? token,RenderArgs dst,RenderArgs src,System.Drawing.Rectangle[] rois,int start,int length)
    {
        for(int i=start;i<start+length;i++)
        for(int y=rois[i].Top;y<rois[i].Bottom;y++) for(int x=rois[i].Left;x<rois[i].Right;x++) {
            var c=src.Surface[x,y];
            dst.Surface[x,y]=ColorBgra.FromBgra((byte)(255-c.B),(byte)(255-c.G),(byte)(255-c.R),c.A);
        }
    }
}

// Asks for the enum names in OnCreatePropertyCollection, as Gaussian Blur+ does, and names its choices.
public sealed class ClassicLocalized:PropertyBasedEffect
{
    public enum Channels {All,Nothing}
    public ClassicLocalized():base("Fixture Classic Localized",(System.Drawing.Image)null!,"Fixture",new EffectOptions {Flags=EffectFlags.Configurable}) {}
    protected override PropertyCollection OnCreatePropertyCollection()
    {
        var names=Services.GetService<IEnumLocalizerFactory>()!.Create(typeof(Channels)).GetLocalizedEnumValues();
        if(names.Count!=2 || names[0].LocalizedName!="All" || !Channels.Nothing.Equals(names[1].EnumValue))throw new InvalidOperationException("enum names");
        // ... and a palette, as Text Fun Factory does.
        var palettes=Services.GetService<IPalettesService>()!;
        if(palettes.CurrentPalette.Count==0 || palettes.DefaultPalette.Count==0)throw new InvalidOperationException("palette");
        return new([StaticListChoiceProperty.CreateForEnum("Channels",Channels.All,false)]);
    }
    protected override ControlInfo OnCreateConfigUI(PropertyCollection properties)
    {
        var ui=CreateDefaultConfigUI(properties);
        ui.SetPropertyControlValue("Channels",ControlInfoPropertyNames.DisplayName,"Fixture-Kanäle");
        ui.FindControlForPropertyName("Channels")!.SetValueDisplayName(Channels.All,"Alle Kanäle");
        return ui;
    }
    protected override void OnRender(System.Drawing.Rectangle[] rois,int start,int length)
    {
        for(int i=start;i<start+length;i++)
        for(int y=rois[i].Top;y<rois[i].Bottom;y++) for(int x=rois[i].Left;x<rois[i].Right;x++) {
            var c=SrcArgs.Surface[x,y];
            DstArgs.Surface[x,y]=ColorBgra.FromBgra((byte)(255-c.B),(byte)(255-c.G),(byte)(255-c.R),c.A);
        }
    }
}

// Reads the clipboard in OnSetRenderInfo, as Paste Alpha does. No image on it is a normal case.
public sealed class ClassicClipboard:Effect
{
    public ClassicClipboard():base("Fixture Classic Clipboard",(System.Drawing.Image)null!,"Fixture",new EffectOptions()) {}
    protected override void OnSetRenderInfo(EffectConfigToken? token,RenderArgs dst,RenderArgs src)
    {
        using var surface=Services.GetService<IClipboardService>()!.TryGetSurface();
        base.OnSetRenderInfo(token,dst,src);
    }
    public override void Render(EffectConfigToken? token,RenderArgs dst,RenderArgs src,System.Drawing.Rectangle[] rois,int start,int length)
    {
        for(int i=start;i<start+length;i++)
        for(int y=rois[i].Top;y<rois[i].Bottom;y++) for(int x=rois[i].Left;x<rois[i].Right;x++) {
            var c=src.Surface[x,y];
            dst.Surface[x,y]=ColorBgra.FromBgra((byte)(255-c.B),(byte)(255-c.G),(byte)(255-c.R),c.A);
        }
    }
}

// Asks for what PSFilterPdn's dialog asks for when it opens.
internal sealed class BitmapServices:PropertyBasedBitmapEffect
{
    IBitmapSource<ColorBgra32>? source;
    public BitmapServices():base("Fixture Services","Fixture",BitmapEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info)
    {
        if(!Directory.Exists(Services.GetService<IUserFilesService>()!.UserFilesPath))throw new InvalidOperationException("user files");
        var files=Services.GetService<IFileSystemService>()!;
        var effects=files.GetPluginDirectoryInfos(PluginType.Effect).Select(i=>i.Path).ToArray();
        if(!files.GetPluginDirectoryRoots().Any() || effects.Length==0 || !effects[0].EndsWith("Effects") || !Directory.Exists(effects[0]))throw new InvalidOperationException("plugin folders");
        if(Services.GetService<IExceptionDialogService>()==null || Services.GetService<IShellService>()==null)throw new InvalidOperationException("dialog or shell");
        Checkerboard();
        source=Environment.GetSourceBitmapBgra32();
        base.OnInitializeRenderInfo(info);
    }
    unsafe void Checkerboard()
    {
        var board=Services.GetService<IVisualStylingService>()!.Checkerboard;
        var size=board.GetRepeatingBitmapSize(CheckerboardTileSize.Small);
        var buffer=new ColorBgr32[size.Width*size.Height];
        fixed(ColorBgr32* start=buffer)board.Render(new RegionPtr<ColorBgr32>(start,size.Width,size.Height,size.Width*4),new Point2Int32(0,0),CheckerboardTileSize.Small);
        if(size.Width<4 || buffer[0].R==buffer[size.Width/2].R)throw new InvalidOperationException("checkerboard");
    }
    protected override void OnRender(IBitmapEffectOutput output)
    {
        using var target=output.LockBgra32();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=ColorBgra32.FromBgra((byte)(255-c.B),(byte)(255-c.G),(byte)(255-c.R),c.A);
        }
    }
}

// Needs its own library of a name paint.net also ships, as a plugin with its own ComputeSharp does.
internal sealed class BitmapOwnLibrary:PropertyBasedBitmapEffect
{
    IBitmapSource<ColorBgra32>? source;
    public BitmapOwnLibrary():base("Fixture Own Library","Fixture",BitmapEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info) {source=Environment.GetSourceBitmapBgra32();base.OnInitializeRenderInfo(info);}
    protected override void OnRender(IBitmapEffectOutput output)
    {
        int mask=K4os.Compression.LZ4.FixtureLibrary.Mask;
        using var target=output.LockBgra32();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=ColorBgra32.FromBgra((byte)(c.B^mask),(byte)(c.G^mask),(byte)(c.R^mask),c.A);
        }
    }
}

internal sealed class FormToken:EffectConfigToken
{
    public int Value=255;
    public FormToken() {}
    FormToken(FormToken other):base(other) {Value=other.Value;}
    public override object Clone()=>new FormToken(this);
}
internal sealed class FixtureForm:EffectConfigForm<BitmapForm,FormToken>
{
    readonly TextBox input=new() {Dock=DockStyle.Top};
    public FixtureForm()
    {
        var ok=new Button {Text="OK",DialogResult=DialogResult.OK,Dock=DockStyle.Bottom};
        Controls.Add(input);Controls.Add(ok);AcceptButton=ok;
        input.TextChanged+=(_,_)=>UpdateTokenFromDialog();
    }
    protected override FormToken OnCreateInitialToken()=>new();
    protected override void OnUpdateDialogFromToken(FormToken token)
    {
        input.Text=token.Value.ToString();
        Console.Error.WriteLine("form-filled:"+token.Value); // what the checks look for
    }
    protected override void OnUpdateTokenFromDialog(FormToken token) {if(int.TryParse(input.Text,out int value))token.Value=value;}
}
internal sealed class BitmapForm:BitmapEffect<FormToken>
{
    IBitmapSource<ColorBgra32>? source;
    public BitmapForm():base("Fixture Bitmap Form","Fixture",BitmapEffectOptions.Create() with {IsConfigurable=true}) {}
    protected override IEffectConfigForm OnCreateConfigForm()=>new FixtureForm();
    protected override void OnInitializeRenderInfo(IBitmapEffectRenderInfo info) {source=Environment.GetSourceBitmapBgra32();}
    protected override void OnRender(IBitmapEffectOutput output)
    {
        int mask=Token!.Value;
        using var target=output.LockBgra32();
        source!.CopyPixels(target,output.Bounds.Location);
        var pixels=target.AsRegionPtr();
        for(int y=0;y<pixels.Height;y++) for(int x=0;x<pixels.Width;x++) {
            var c=pixels[x,y];
            pixels[x,y]=ColorBgra32.FromBgra((byte)(c.B^mask),(byte)(c.G^mask),(byte)(c.R^mask),c.A);
        }
    }
}

internal sealed class GpuImage:PropertyBasedGpuImageEffect
{
    public GpuImage():base("Fixture GPU Image","Fixture",GpuImageEffectOptions.Create() with {IsConfigurable=true}) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([new BooleanProperty("Enabled",true)]);
    protected override IDeviceImage OnCreateOutput(IDeviceContext dc)=>
        Token!.GetProperty<BooleanProperty>("Enabled")!.Value ? new InvertEffect(dc,Environment.SourceImage) : Environment.SourceImage;
}

// paint.net's own Direct2D effect stands for plugin shaders: a pixel shader registered as a custom effect.
internal sealed class GpuShader:PropertyBasedGpuImageEffect
{
    public GpuShader():base("Fixture GPU Shader","Fixture",GpuImageEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override IDeviceImage OnCreateOutput(IDeviceContext dc)
    {
        var effect=new PdnBrightnessContrastEffect(dc);
        effect.Properties.Input.Set(Environment.SourceImage);
        effect.Properties.Brightness.SetValue(-1f);
        return effect;
    }
}

internal sealed class GpuDrawing:PropertyBasedGpuDrawingEffect
{
    public GpuDrawing():base("Fixture GPU Drawing","Fixture",GpuDrawingEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override void OnDraw(IDeviceContext dc)
    {
        using var brush=dc.CreateSolidColorBrush(new ColorRgba128Float(0,0,0,1));
        dc.FillRectangle(new RectFloat(0,0,Environment.Document.Size.Width,Environment.Document.Size.Height),brush);
    }
}

// Long enough to outlast every test: only cancelling the process ends these.
internal sealed class BitmapSlow:PropertyBasedBitmapEffect
{
    public BitmapSlow():base("Fixture Slow Bitmap","Fixture",BitmapEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override void OnRender(IBitmapEffectOutput output)=>Thread.Sleep(120000);
}
internal sealed class GpuSlow:PropertyBasedGpuImageEffect
{
    public GpuSlow():base("Fixture Slow GPU","Fixture",GpuImageEffectOptions.Create()) {}
    protected override PropertyCollection OnCreatePropertyCollection()=>new([]);
    protected override IDeviceImage OnCreateOutput(IDeviceContext dc) {Thread.Sleep(120000);return Environment.SourceImage;}
}
