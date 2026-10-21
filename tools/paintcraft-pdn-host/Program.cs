using System.Reflection;
using System.IO;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Runtime.Loader;
using PaintDotNet;
using PaintDotNet.Effects;
using PaintDotNet.PropertySystem;

internal static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        var dir = Environment.GetEnvironmentVariable("PAINTDOTNET_DIR") ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "paint.net");
        AssemblyLoadContext.Default.Resolving += (c,n) => {
            var path=Path.Combine(dir,n.Name+".dll");
            return File.Exists(path) ? c.LoadFromAssemblyPath(path) : null;
        };
        // Paint.NET native support libraries are resolved from the user's installation.
        Environment.SetEnvironmentVariable("PATH", dir+Path.PathSeparator+Environment.GetEnvironmentVariable("PATH"));
        try {
            using var watchdog = HostWatchdog.Start(args);
            byte[] data=Array.Empty<byte>();
            if(!args.Contains("--list") && !args.Contains("--inspect") && !args.Contains("--catalog")) {
                using var source=Console.OpenStandardInput();
                using var buffer=new MemoryStream();
                var block=new byte[65536];int n;
                while((n=source.Read(block))!=0) {
                    if(buffer.Length+n>500_000_024) throw new InvalidDataException("Image exceeds host limit");
                    buffer.Write(block,0,n);
                }
                data=buffer.ToArray();
            }
            if(args.Contains("--catalog")) return PluginCatalog.Scan(args,dir);
            if(args.Contains("--inspect")) return PluginCatalog.Inspect(args[0]);
            if(args.Contains("--decode") || args.Contains("--encode")) return FileTypeBridge.Run(args,data);
            return Run(args,data);
        }
        catch(Exception e) { Console.Error.WriteLine(e.ToString()); return 1; }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static int Run(string[] args, byte[] data)
    {
        if(args.Length<1) throw new ArgumentException("Usage: PaintCraft.PdnHost plugin.dll [effect-type] [--defaults]");
        var plugin=Path.GetFullPath(args[0]);
        var pluginDir=Path.GetDirectoryName(plugin)!;
        AssemblyLoadContext.Default.Resolving += (c,n) => {
            var path=Path.Combine(pluginDir,n.Name+".dll");
            return File.Exists(path) ? c.LoadFromAssemblyPath(path) : null;
        };
        var assembly=AssemblyLoadContext.Default.LoadFromAssemblyPath(plugin);
        Type[] types;
        try { types=assembly.GetExportedTypes(); }
        catch(ReflectionTypeLoadException e) { types=e.Types.Where(t=>t!=null).Cast<Type>().ToArray(); }
        var effects=types.Where(t=>!t.IsAbstract && typeof(Effect).IsAssignableFrom(t) && t.GetConstructor(Type.EmptyTypes)!=null).ToArray();
        if(args.Contains("--list")) { foreach(var t in effects) Console.WriteLine(t.FullName); return 0; }
        if(effects.Length==0) throw new NotSupportedException("Diese DLL enthält keinen klassischen Paint.NET-CPU-Effekt. GPU-Effekte und Dateiformat-Plugins benötigen andere Host-APIs.");
        Type? selected=null;
        if(args.Length>1 && !args[1].StartsWith("--")) selected=effects.Single(t=>t.FullName==args[1]);
        else if(effects.Length==1) selected=effects[0];
        else {
            Application.EnableVisualStyles();
            using var form=new Form {Text="PaintCraft – Paint.NET-Effekt wählen",Width=560,Height=400,StartPosition=FormStartPosition.CenterScreen};
            var list=new ListBox {Dock=DockStyle.Fill,DisplayMember="Name",DataSource=effects};
            var ok=new Button {Text="Öffnen",Dock=DockStyle.Bottom,DialogResult=DialogResult.OK};
            form.Controls.Add(list);form.Controls.Add(ok);form.AcceptButton=ok;
            if(form.ShowDialog()!=DialogResult.OK) return 2;
            selected=(Type)list.SelectedItem!;
        }
        using var input=new BinaryReader(new MemoryStream(data,false));
        if(input.ReadUInt32()!=0x58464350) throw new InvalidDataException("Invalid PCFX header");
        int w=input.ReadInt32(),h=input.ReadInt32();
        if(w<1 || h<1 || w>65535 || h>65535 || (long)w*h>100_000_000) throw new InvalidDataException("Image dimensions exceed host limit");
        var primary=ColorBgra.FromUInt32(input.ReadUInt32());
        var secondary=ColorBgra.FromUInt32(input.ReadUInt32());
        float brush=input.ReadSingle();
        using var src=new Surface(w,h);
        using var dst=new Surface(w,h);
        var row=new byte[checked(w*4)];
        for(int y=0;y<h;y++) {
            input.BaseStream.ReadExactly(row);
            unsafe {row.AsSpan().CopyTo(new Span<byte>(src.GetRowPointer(y),row.Length));}
        }
        var mask=new byte[checked(w*h)];input.BaseStream.ReadExactly(mask);
        using var region=new PdnRegion();region.MakeEmpty();
        for(int y=0;y<h;y++) for(int x=0;x<w;) {
            while(x<w && mask[y*w+x]==0)x++;
            int start=x;while(x<w && mask[y*w+x]!=0)x++;
            if(start<x)region.Union(new Rectangle(start,y,x-start,1));
        }
        dst.CopySurface(src);
        using var effect=(Effect)Activator.CreateInstance(selected!)!;
        #pragma warning disable CS0618 // This bridge intentionally hosts the classic API.
        effect.EnvironmentParameters=(EffectEnvironmentParameters)Activator.CreateInstance(typeof(EffectEnvironmentParameters),primary,secondary,brush,region,src)!;
        #pragma warning restore CS0618
        EffectConfigToken? token=null;
        if(effect is PropertyBasedEffect property) token=new PropertyBasedEffectConfigToken(property.CreatePropertyCollection());
        if(token is PropertyBasedEffectConfigToken propertyToken && (!args.Contains("--defaults") || args.Contains("--validate-dialog"))) {
            using var dialog=new Form {Text="PaintCraft – "+selected!.Name,Width=580,Height=600,StartPosition=FormStartPosition.CenterScreen};
            var grid=new PropertyGrid {Dock=DockStyle.Fill,SelectedObject=new PluginProperties(propertyToken.Properties),ToolbarVisible=false};
            var buttons=new FlowLayoutPanel {Dock=DockStyle.Bottom,Height=40,FlowDirection=FlowDirection.RightToLeft};
            var ok=new Button {Text="Anwenden",DialogResult=DialogResult.OK};
            var cancel=new Button {Text="Abbrechen",DialogResult=DialogResult.Cancel};
            buttons.Controls.Add(cancel);buttons.Controls.Add(ok);dialog.Controls.Add(grid);dialog.Controls.Add(buttons);
            dialog.AcceptButton=ok;dialog.CancelButton=cancel;
            if(!args.Contains("--validate-dialog") && dialog.ShowDialog()!=DialogResult.OK)return 2;
        } else if(token is not PropertyBasedEffectConfigToken && (!args.Contains("--defaults") || args.Contains("--validate-dialog"))) {
            using var dialog=effect.CreateConfigDialog();
            if(dialog!=null) {
                dialog.Effect=effect;
                if(token!=null)dialog.EffectToken=token;
                if(!args.Contains("--validate-dialog") && dialog.ShowDialog()!=DialogResult.OK)return 2;
                token=dialog.EffectToken;
            }
        }
        using var sa=new RenderArgs(src);using var da=new RenderArgs(dst);
        effect.SetRenderInfo(token,da,sa);
        var scans=region.GetRegionScansReadOnlyInt();
        effect.Render(token,da,sa,scans,0,scans.Length);
        // No result bytes are published until the plugin finished successfully.
        using var output=new BinaryWriter(Console.OpenStandardOutput());
        output.Write(0x58464350u);output.Write(w);output.Write(h);
        for(int y=0;y<h;y++) {
            unsafe {new ReadOnlySpan<byte>(dst.GetRowPointer(y),row.Length).CopyTo(row);}
            output.Write(row);
        }
        return 0;
    }
}

// Paint.NET's IndirectUI is initialized by its application, not its public plugin API.
// Present typed PropertySystem values without relying on private initialization hooks.
sealed class PluginProperties(PropertyCollection values) : CustomTypeDescriptor
{
    public override PropertyDescriptorCollection GetProperties()=>GetProperties(null);
    public override PropertyDescriptorCollection GetProperties(Attribute[]? attributes)=>new(values.Select(p=>(PropertyDescriptor)new PluginProperty(p)).ToArray());
    public override object GetPropertyOwner(PropertyDescriptor? pd)=>this;
}
sealed class PluginProperty(Property value) : PropertyDescriptor(value.Name,Array.Empty<Attribute>())
{
    public override Type ComponentType=>typeof(PluginProperties);
    public override Type PropertyType=>value.ValueType;
    public override bool IsReadOnly=>value.ReadOnly;
    public override bool CanResetValue(object component)=>!value.ReadOnly;
    public override object? GetValue(object? component)=>value.Value;
    public override void SetValue(object? component,object? newValue){value.Value=newValue!;OnValueChanged(component,EventArgs.Empty);}
    public override void ResetValue(object component)=>SetValue(component,value.DefaultValue);
    public override bool ShouldSerializeValue(object component)=>false;
    public override TypeConverter Converter=>value is StaticListChoiceProperty choices ? new ChoicesConverter(choices.ValueChoices) : base.Converter;
}
sealed class ChoicesConverter(object[] choices) : TypeConverter
{
    public override bool GetStandardValuesSupported(ITypeDescriptorContext? context)=>true;
    public override bool GetStandardValuesExclusive(ITypeDescriptorContext? context)=>true;
    public override StandardValuesCollection GetStandardValues(ITypeDescriptorContext? context)=>new(choices);
    public override bool CanConvertFrom(ITypeDescriptorContext? context,Type sourceType)=>sourceType==typeof(string);
    public override object? ConvertFrom(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object value)=>choices.FirstOrDefault(x=>x.ToString()==value.ToString()) ?? base.ConvertFrom(context,culture,value);
}
