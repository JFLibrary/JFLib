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
            return !((show || validate) && configurable && any) || Dialogs.Properties(effect.EffectInfo.Name,properties,show);
        }
        token=null;
        if(!configurable)return true;
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
    object? settings,direct2D,imaging,directWrite;

    public object? GetService(Type t)
    {
        if(t==typeof(ISettingsService))return settings??=ClosedInterface.Implement(t,(_,a)=>Setting((string)a[0]!));
        if(t==typeof(PaintDotNet.Direct2D1.IDirect2DFactory))return direct2D??=PaintDotNet.Direct2D1.Direct2DFactory.Create();
        if(t==typeof(IImagingFactory))return imaging??=ImagingFactory.CreateRef();
        if(t==typeof(PaintDotNet.DirectWrite.IDirectWriteFactory))return directWrite??=PaintDotNet.DirectWrite.DirectWriteFactory.CreateRef();
        return null;
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
