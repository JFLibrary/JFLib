using System.IO;
using System.Reflection;
using System.Reflection.Emit;
using PaintDotNet;
using PaintDotNet.AppModel;
using PaintDotNet.Effects;
using PaintDotNet.Effects.Gpu;
using PaintDotNet.Imaging;
using PaintDotNet.PropertySystem;
using PaintDotNet.Rendering;

// The v5 effect API: BitmapEffect, GpuEffect and their PropertyBased forms. Both kinds render
// through the public IBitmapEffectRenderer; a GPU effect creates its own Direct2D device behind it.
internal static class ModernEffects
{
    internal static bool Handles(Type t)=>!typeof(Effect).IsAssignableFrom(t);

    internal static int Run(Type type,EffectInput input,string[] args)
    {
        bool gpu=typeof(GpuEffect).IsAssignableFrom(type);
        int choice=Array.IndexOf(args,"--gpu");
        bool software=choice>=0 && choice+1<args.Length && args[choice+1]=="software";
        var environment=input.Environment();
        // Plugins may read Environment while building their properties, so this instance is initialized too.
        using var config=(IEffect)Activator.CreateInstance(type,true)!;
        var initialize=typeof(IEffect).GetMethod("Initialize",BindingFlags.Instance|BindingFlags.NonPublic);
        initialize?.Invoke(config,[new HostServices(software),environment]);
        if(!Configure(config,type,args,out var token))return 2;
        if(args.Contains("--validate-dialog") && !input.HasPixels)return 0;
        for(;;) {
            try {
                using var effect=config.EffectInfo.CreateInstance(new HostServices(software),environment);
                Render(effect,token,input,gpu);
                return 0;
            }
            // Drivers can refuse a device (remote sessions, old GPUs); WARP renders the same graph.
            catch(Exception e) when(gpu && !software && e.GetBaseException() is not (MissingMemberException or TypeLoadException)) {
                Console.Error.WriteLine("GPU-Rendering fehlgeschlagen, weiter in Software: "+e.GetBaseException().Message);
                software=true;
            }
        }
    }

    static bool Configure(IEffect effect,Type type,string[] args,out EffectConfigToken? token)
    {
        bool validate=args.Contains("--validate-dialog");
        bool show=!args.Contains("--defaults") && !validate;
        bool configurable=(effect.Options as EffectOptionsBase)?.IsConfigurable ?? true;
        token=null;
        // Such an effect runs without a token and may not even have properties to ask for.
        if(!configurable)return true;
        var properties=effect switch {
            PropertyBasedBitmapEffect e=>e.CreatePropertyCollection(),
            PropertyBasedGpuImageEffect e=>e.CreatePropertyCollection(),
            PropertyBasedGpuDrawingEffect e=>e.CreatePropertyCollection(),
            PropertyBasedGpuEffect e=>e.CreatePropertyCollection(),
            _=>null
        };
        if(properties!=null) {
            token=new PropertyBasedEffectConfigToken(properties);
            bool any=properties.Any(p=>!p.ReadOnly);
            return !((show || validate) && any) || Dialogs.Properties(effect.EffectInfo.Name,properties,ParameterLabels.For(effect,properties),show);
        }
        using var form=effect.CreateConfigForm();
        // Only this hands the form its effect and initial token; setting it fills the controls.
        form.SetEffect(effect);
        form.Token=form.Token;
        if(show && form.ShowDialog(null)!=System.Windows.Forms.DialogResult.OK)return false;
        token=form.Token;
        return true;
    }

    static unsafe void Render(IEffect effect,EffectConfigToken? token,EffectInput input,bool gpu)
    {
        using var renderer=effect.CreateRenderer<IBitmapEffectRenderer>();
        renderer.Initialize(new BitmapEffectInitializeInfo());
        renderer.SetToken(token);
        var info=renderer.RenderInfo;
        var dst=input.Dst;
        var tiles=Tiles(input.Bounds,gpu,info.Schedule);
        int stride=dst.Stride;
        void Tile(RectInt32 r) {
            var format=PixelFormats.Bgra32;
            var start=(byte*)dst.GetRowPointer(r.Y)+r.X*4L;
            if(!gpu && !info.Flags.HasFlag(BitmapEffectRenderingFlags.UninitializedOutputBuffer))
                for(int y=0;y<r.Height;y++)new Span<byte>(start+y*(long)stride,r.Width*4).Clear();
            renderer.Render(start,stride,(nint)(stride*(r.Height-1L)+r.Width*4L),in format,r);
        }
        if(tiles.Count==0)return;
        if(gpu || info.Flags.HasFlag(BitmapEffectRenderingFlags.SingleThreaded)) {foreach(var t in tiles)Tile(t);return;}
        int first=info.Flags.HasFlag(BitmapEffectRenderingFlags.FirstTileIsRenderedWithBarrier)?1:0;
        for(int i=0;i<first;i++)Tile(tiles[i]);
        Parallel.For(first,tiles.Count,i=>Tile(tiles[i]));
    }

    static List<RectInt32> Tiles(System.Drawing.Rectangle bounds,bool gpu,BitmapEffectRenderingSchedule schedule)
    {
        var (width,height)=gpu ? (2048,2048) : schedule switch {
            BitmapEffectRenderingSchedule.None=>(bounds.Width,bounds.Height),
            BitmapEffectRenderingSchedule.HorizontalStrips=>(bounds.Width,64),
            _=>(256,256)
        };
        var tiles=new List<RectInt32>();
        for(int y=bounds.Top;y<bounds.Bottom;y+=height) for(int x=bounds.Left;x<bounds.Right;x+=width)
            tiles.Add(new RectInt32(x,y,Math.Min(width,bounds.Right-x),Math.Min(height,bounds.Bottom-y)));
        return tiles;
    }
}

// What effects ask their IServiceProvider for outside paint.net. Anything else stays null, so a
// plugin that needs another application service fails with paint.net's own "service not found".
internal sealed class HostServices(bool software):IServiceProvider
{
    const string RenderingGpu="Graphics/RenderingGpuChoice";
    static bool cacheReady;
    object? settings,direct2D,imaging,directWrite,enumNames,clipboard,palettes,userFiles,fileSystem,errors,shell,styling;
    // Paint.NET's own files stay as they are: a plugin's settings land here.
    static string DataFolder(string name)
    {
        var dir=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"PaintCraft","pdn-host",name);
        Directory.CreateDirectory(dir);
        return dir;
    }

    public object? GetService(Type t)
    {
        if(t==typeof(IEnumLocalizerFactory))return enumNames??=ClosedInterface.Implement(t,(_,a)=>EnumNames((Type)a[0]!));
        if(t==typeof(IClipboardService))return clipboard??=Clipboard();
        if(t==typeof(IPalettesService))return palettes??=Palettes();
        if(t==typeof(IVisualStylingService))return styling??=VisualStyling();
        if(t==typeof(IUserFilesService))return userFiles??=ClosedInterface.Implement(t,(member,_)=>member is "get_UserFilesPath" or "TryGetUserFilesPath" ? DataFolder("User Files") : null);
        if(t==typeof(IFileSystemService))return fileSystem??=ClosedInterface.Implement(t,(member,_)=>member switch {
            "get_PerUserAppFilesPath"=>DataFolder("App Files"),
            "GetPluginDirectoryRoots"=>PluginRoots(),
            "GetPluginDirectoryInfos"=>PluginFolders(),
            _=>null
        });
        if(t==typeof(IExceptionDialogService))return errors??=ClosedInterface.Implement(t,(member,a)=>member=="ShowErrorDialog" ? ShowError(a) : null);
        if(t==typeof(IShellService))return shell??=ClosedInterface.Implement(t,(member,a)=>member.StartsWith("Launch") ? Open((string)a[1]!) : null);
        if(t==typeof(ISettingsService))return settings??=ClosedInterface.Implement(t,(_,a)=>Setting((string)a[0]!));
        if(t==typeof(PaintDotNet.Direct2D1.IDirect2DFactory))return direct2D??=PaintDotNet.Direct2D1.Direct2DFactory.Create();
        if(t==typeof(IImagingFactory))return imaging??=ImagingFactory.CreateRef();
        if(t==typeof(PaintDotNet.DirectWrite.IDirectWriteFactory))return directWrite??=PaintDotNet.DirectWrite.DirectWriteFactory.CreateRef();
        return null;
    }

    // paint.net translates its own enums; a plugin's values keep their names, as without a translation.
    static object EnumNames(Type type)
    {
        object Name(object value)=>ClosedInterface.Implement(typeof(ILocalizedEnumValue),(member,_)=>member switch {
            "get_EnumType"=>type,
            "get_EnumValue"=>value,
            "get_LocalizedName"=>value.ToString(),
            _=>null
        });
        return ClosedInterface.Implement(typeof(IEnumLocalizer),(member,a)=>member switch {
            "get_EnumType"=>type,
            "GetLocalizedEnumValue"=>Name(a[0]!),
            "GetLocalizedEnumValues"=>Enum.GetValues(type).Cast<object>().Select(v=>(ILocalizedEnumValue)Name(v)).ToList(),
            _=>null
        });
    }

    // paint.net's own service for effects, given a window: the application always has one, and
    // without it the service retries for 12 seconds before it reports an empty clipboard.
    static object? Clipboard()
    {
        var app=Assembly.Load("paintdotnet");
        // renamed in 5.2
        var type=app.GetType("PaintDotNet.Effects.ClipboardServiceForEffects") ?? app.GetType("PaintDotNet.AppModel.ClipboardServiceForPlugins");
        if(type==null)return null;
        var real=(IClipboardService)Activator.CreateInstance(type,true)!;
        var window=new System.Windows.Forms.NativeWindow();
        window.CreateHandle(new System.Windows.Forms.CreateParams());
        return ClosedInterface.Implement(typeof(IClipboardService),(member,a)=>{
            if(member!="Open")return null;
            var handle=(nint)a[0]!;
            return real.Open(handle==0 ? window.Handle : handle);
        });
    }

    // Previews lay transparent pixels over a checkerboard: the light one, in the usual tile sizes.
    static object VisualStyling()
    {
        static int Shift(object size)=>size is CheckerboardTileSize.Small ? 3 : 4;
        var checkerboard=ClosedInterface.Implement(typeof(ICheckerboardVisualStyling),(member,a)=>{
            if(member=="get_Color1")return SrgbColors.White;
            if(member=="get_Color2")return SrgbColors.Silver;
            if(member=="GetRepeatingBitmapSize")return new SizeInt32(2<<Shift(a[0]!),2<<Shift(a[0]!));
            if(member!="Render")return null;
            var region=(RegionPtr<ColorBgr32>)a[0]!;
            var offset=(Point2Int32)a[1]!;
            int shift=Shift(a[2]!);
            for(int y=0;y<region.Height;y++) for(int x=0;x<region.Width;x++) {
                byte level=(byte)((((x+offset.X)>>shift)+((y+offset.Y)>>shift)&1)==0 ? 255 : 192);
                region[x,y]=ColorBgr32.FromBgr(level,level,level);
            }
            return null;
        });
        var canvas=ClosedInterface.Implement(typeof(ICanvasVisualStyling),(member,_)=>member=="get_BackgroundColor" ? SrgbColors.Gray : null);
        return ClosedInterface.Implement(typeof(IVisualStylingService),(member,_)=>member switch {
            "get_Checkerboard"=>checkerboard,
            "get_Canvas"=>canvas,
            _=>null
        });
    }

    static List<string> PluginRoots()
    {
        var docs=Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);
        var roots=new List<string> {Path.Combine(docs,"PaintCraft App Files"),Path.Combine(docs,"Paint.NET App Files"),Program.Install};
        // The running plugin's own root first, wherever PaintCraft found it.
        var folder=Path.GetDirectoryName(PluginCatalog.LoadedFrom);
        while(folder!=null && Path.GetFileName(folder) is not ("Effects" or "FileTypes"))folder=Path.GetDirectoryName(folder);
        if(folder!=null)roots.Insert(0,Path.GetDirectoryName(folder)!);
        return roots.Distinct(StringComparer.OrdinalIgnoreCase).ToList();
    }

    static List<PluginDirectoryInfo> PluginFolders()
    {
        var folders=new List<PluginDirectoryInfo>();
        // internal, and 5.2 added an install category to it: filled by type, the rest stays default
        var create=typeof(PluginDirectoryInfo).GetConstructors(BindingFlags.Instance|BindingFlags.NonPublic).Single();
        foreach(var root in PluginRoots()) foreach(var (type,name) in new[]{(PluginType.Effect,"Effects"),(PluginType.FileType,"FileTypes"),(PluginType.Shape,"Shapes")}) {
            var path=Path.Combine(root,name);
            folders.Add((PluginDirectoryInfo)create.Invoke(create.GetParameters().Select(p=>p.ParameterType==typeof(PluginType) ? type : p.ParameterType==typeof(string) ? path : Activator.CreateInstance(p.ParameterType)).ToArray()));
        }
        return folders;
    }

    static object? ShowError(object?[] a)
    {
        var text=string.Join("\n\n",a.Skip(1).Select(x=>x is Exception e ? e.GetBaseException().Message : x as string).Where(x=>!string.IsNullOrEmpty(x)));
        Console.Error.WriteLine(text);
        if(Dialogs.Interactive)System.Windows.Forms.MessageBox.Show(a[0] as System.Windows.Forms.IWin32Window,text,"PaintCraft – Plugin",System.Windows.Forms.MessageBoxButtons.OK,System.Windows.Forms.MessageBoxIcon.Error);
        return null;
    }

    static object Open(string target)
    {
        try {System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(target) {UseShellExecute=true})?.Dispose();return true;}
        catch(Exception e) when(e is System.ComponentModel.Win32Exception or InvalidOperationException) {return false;}
    }

    // paint.net's palette service hangs on its user files. A plugin gets a plain palette of the usual
    // 96 colors instead: a gray ramp, then saturated hues.
    static object Palettes()
    {
        var colors=new List<ColorBgra>();
        for(int i=0;i<16;i++)colors.Add(ColorBgra.FromBgr((byte)(i*17),(byte)(i*17),(byte)(i*17)));
        for(int i=0;i<80;i++) {
            double h=i*6.0/80,f=h-Math.Floor(h);
            byte up=(byte)(255*f),down=(byte)(255*(1-f));
            colors.Add((int)h switch {
                0=>ColorBgra.FromBgr(0,up,255),
                1=>ColorBgra.FromBgr(0,255,down),
                2=>ColorBgra.FromBgr(up,255,0),
                3=>ColorBgra.FromBgr(255,down,0),
                4=>ColorBgra.FromBgr(255,0,up),
                _=>ColorBgra.FromBgr(down,0,255)
            });
        }
        IReadOnlyList<ColorBgra> palette=colors;
        return ClosedInterface.Implement(typeof(IPalettesService),(member,_)=>member is "get_DefaultPalette" or "get_CurrentPalette" ? palette : null);
    }

    object Setting(string path)
    {
        if(path==AppSettingPaths.Effects.DefaultQualityLevel || path==AppSettingPaths.Effects.DefaultQualityLevelGpu)
            return Value(typeof(IScalarAppSetting<>).MakeGenericType(typeof(int)),path,typeof(int),2);
        if(path!=RenderingGpu)throw new NotSupportedException("Die paint.net-Einstellung "+path+" gibt es im Host nicht.");
        var property=typeof(GpuEffect).GetProperty("RenderingGpuChoice",BindingFlags.Instance|BindingFlags.NonPublic)
            ?? throw new MissingMemberException(nameof(GpuEffect),"RenderingGpuChoice");
        var choice=property.PropertyType.GetGenericArguments()[0];
        if(!software)PrepareGpuCache(choice.Assembly);
        var value=choice.GetField(software?"Software":"Default")?.GetValue(null) ?? throw new MissingFieldException(choice.FullName,"Default");
        return Value(typeof(ISetting<>).MakeGenericType(choice),path,choice,value);
    }

    static object Value(Type setting,string path,Type type,object value)=>ClosedInterface.Implement(setting,(member,_)=>member switch {
        "get_Path"=>path,
        "get_ValueType"=>type,
        "get_IsReadOnly"=>true,
        "get_Value" or "get_DefaultValue"=>value,
        _=>null
    });

    // The default GPU choice looks up adapter facts in a cache only paint.net's startup opens.
    static void PrepareGpuCache(Assembly framework)
    {
        if(cacheReady)return;
        var service=framework.GetType("PaintDotNet.Gpu.GpuInfoCacheService",true)!;
        var initialize=service.GetMethod("Initialize",BindingFlags.Public|BindingFlags.Static) ?? throw new MissingMethodException(service.FullName,"Initialize");
        var dir=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"PaintCraft","pdn-host");
        Directory.CreateDirectory(dir);
        initialize.Invoke(null,[ClosedInterface.Implement(initialize.GetParameters()[0].ParameterType,(_,_)=>dir)]);
        cacheReady=true;
    }
}

// What a plugin says about its parameters. IndirectUI describes them once it knows where its controls
// live; the controls themselves need paint.net's application, so the host only reads the description.
internal static class ParameterLabels
{
    static bool ready;

    internal static PaintDotNet.IndirectUI.ControlInfo? For(object effect,PropertyCollection properties)
    {
        try {
            if(!ready) {
                typeof(PaintDotNet.IndirectUI.ControlInfo).GetMethod("Initialize",BindingFlags.Static|BindingFlags.NonPublic)?.Invoke(null,[Assembly.Load("paintdotnet")]);
                ready=true;
            }
            return effect switch {
                PropertyBasedEffect e=>e.CreateConfigUI(properties),
                PropertyBasedBitmapEffect e=>e.CreateConfigUI(properties),
                PropertyBasedGpuImageEffect e=>e.CreateConfigUI(properties),
                PropertyBasedGpuDrawingEffect e=>e.CreateConfigUI(properties),
                PropertyBasedGpuEffect e=>e.CreateConfigUI(properties),
                _=>null
            };
        }
        // A plugin's UI code may want more of the application: its plain names still work.
        catch(Exception e) {
            Console.Error.WriteLine("Keine Beschriftungen: "+e.GetBaseException().Message);
            return null;
        }
    }
}

// paint.net closes these interfaces with an internal member (IInternalImpl), which neither C# nor
// DispatchProxy can implement. A dynamic assembly may, once it ignores access checks to paint.net.
internal static class ClosedInterface
{
    static readonly ModuleBuilder module=CreateModule();
    static int count;

    static ModuleBuilder CreateModule()
    {
        var names=Directory.GetFiles(Program.Install,"PaintDotNet*.dll").Select(Path.GetFileNameWithoutExtension);
        var attribute=typeof(System.Runtime.CompilerServices.IgnoresAccessChecksToAttribute).GetConstructor([typeof(string)])!;
        var assembly=AssemblyBuilder.DefineDynamicAssembly(new AssemblyName("PaintCraft.PdnHost.Shims"),AssemblyBuilderAccess.Run,
            names.Select(n=>new CustomAttributeBuilder(attribute,[n])));
        return assembly.DefineDynamicModule("Shims");
    }

    internal static object Implement(Type face,Func<string,object?[],object?> handler)
    {
        var type=module.DefineType("Shim"+ ++count,TypeAttributes.Public|TypeAttributes.Sealed,typeof(object),[face]);
        var field=type.DefineField("members",typeof(Func<object?[],object?>[]),FieldAttributes.Public);
        type.DefineDefaultConstructor(MethodAttributes.Public);
        var members=new List<Func<object?[],object?>>();
        foreach(var i in face.GetInterfaces().Append(face))
        foreach(var m in i.GetMethods(BindingFlags.Instance|BindingFlags.Public|BindingFlags.NonPublic).Where(m=>m.IsAbstract)) {
            var parameters=m.GetParameters().Select(p=>p.ParameterType).ToArray();
            var method=type.DefineMethod(i.Name+"."+m.Name,MethodAttributes.Private|MethodAttributes.Virtual|MethodAttributes.Final|MethodAttributes.HideBySig|MethodAttributes.NewSlot,m.ReturnType,parameters);
            var il=method.GetILGenerator();
            il.Emit(OpCodes.Ldarg_0);il.Emit(OpCodes.Ldfld,field);il.Emit(OpCodes.Ldc_I4,members.Count);il.Emit(OpCodes.Ldelem_Ref);
            il.Emit(OpCodes.Ldc_I4,parameters.Length);il.Emit(OpCodes.Newarr,typeof(object));
            for(int k=0;k<parameters.Length;k++) {
                il.Emit(OpCodes.Dup);il.Emit(OpCodes.Ldc_I4,k);il.Emit(OpCodes.Ldarg,k+1);
                if(parameters[k].IsValueType)il.Emit(OpCodes.Box,parameters[k]);
                il.Emit(OpCodes.Stelem_Ref);
            }
            il.Emit(OpCodes.Callvirt,typeof(Func<object?[],object?>).GetMethod("Invoke")!);
            if(m.ReturnType==typeof(void))il.Emit(OpCodes.Pop);
            else if(m.ReturnType.IsValueType)il.Emit(OpCodes.Unbox_Any,m.ReturnType);
            else il.Emit(OpCodes.Castclass,m.ReturnType);
            il.Emit(OpCodes.Ret);
            type.DefineMethodOverride(method,m);
            var result=m.ReturnType;
            var name=m.Name;
            members.Add(a=>handler(name,a) ?? (result.IsValueType && result!=typeof(void) ? Activator.CreateInstance(result) : null));
        }
        var instance=Activator.CreateInstance(type.CreateType())!;
        instance.GetType().GetField("members")!.SetValue(instance,members.ToArray());
        return instance;
    }
}

namespace System.Runtime.CompilerServices
{
    // Honoured by the runtime by name; the framework declares it but keeps it internal.
    [AttributeUsage(AttributeTargets.Assembly,AllowMultiple=true)]
    internal sealed class IgnoresAccessChecksToAttribute(string assemblyName):Attribute
    {
        public string AssemblyName { get; }=assemblyName;
    }
}
