using System.Reflection;
using System.IO;
using System.ComponentModel;
using System.Drawing.Design;
using System.Runtime.CompilerServices;
using System.Runtime.Loader;
using PaintDotNet;
using PaintDotNet.Effects;
using PaintDotNet.IndirectUI;
using PaintDotNet.PropertySystem;

internal static class Program
{
    internal static string Install="";

    [STAThread]
    static int Main(string[] args)
    {
        // PaintCraft reads diagnostics as UTF-8; without a console .NET would use the ANSI code page.
        Console.SetError(new StreamWriter(Console.OpenStandardError(),new System.Text.UTF8Encoding(false)) {AutoFlush=true});
        var dir = Environment.GetEnvironmentVariable("PAINTDOTNET_DIR") ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "paint.net");
        Install=dir;
        AssemblyLoadContext.Default.Resolving += (c,n) => {
            var path=Path.Combine(dir,n.Name+".dll");
            return File.Exists(path) ? c.LoadFromAssemblyPath(path) : null;
        };
        // Paint.NET native support libraries are resolved from the user's installation.
        Environment.SetEnvironmentVariable("PATH", dir+Path.PathSeparator+Environment.GetEnvironmentVariable("PATH"));
        try {
            using var watchdog = HostWatchdog.Start(args);
            byte[] data=Array.Empty<byte>();
            if(!args.Contains("--list") && !args.Contains("--inspect") && !args.Contains("--catalog") && !args.Contains("--validate-dialog")) {
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
            WineMode.Apply(dir);
            if(args.Contains("--inspect")) return PluginCatalog.Inspect(args[0]);
            if(args.Contains("--decode") || args.Contains("--encode")) return FileTypeBridge.Run(args,data);
            return Run(args,data);
        }
        catch(Exception e) { Console.Error.WriteLine(Explain(e)); return 1; }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static int Run(string[] args, byte[] data)
    {
        if(args.Length<1) throw new ArgumentException("Usage: PaintCraft.PdnHost plugin.dll [effect-type] [--defaults] [--gpu software]");
        Dialogs.Interactive=!args.Contains("--defaults") && !args.Contains("--validate-dialog");
        var assembly=PluginCatalog.Load(args[0]);
        Type[] types;
        try { types=assembly.GetTypes(); }
        catch(ReflectionTypeLoadException e) { types=e.Types.Where(t=>t!=null).Cast<Type>().ToArray(); }
        var effects=types.Where(t=>typeof(IEffect).IsAssignableFrom(t) && PluginCatalog.CanCreate(t)).ToArray();
        if(args.Contains("--list")) { foreach(var t in effects) Console.WriteLine(t.FullName); return 0; }
        if(effects.Length==0) throw new NotSupportedException("Diese DLL enthält keinen Paint.NET-Effekt.");
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
        using var input=EffectInput.Read(data,args.Contains("--validate-dialog"));
        int code=ModernEffects.Handles(selected!) ? ModernEffects.Run(selected!,input,args) : RunClassic(selected!,input,args);
        // No result bytes are published until the plugin finished successfully.
        if(code==0 && input.HasPixels)input.WriteResult();
        return code;
    }

    static int RunClassic(Type selected,EffectInput input,string[] args)
    {
        using var effect=(Effect)Activator.CreateInstance(selected,true)!;
        effect.Services=new HostServices(false);
        #pragma warning disable CS0618 // This bridge intentionally hosts the classic API.
        effect.EnvironmentParameters=input.Environment();
        #pragma warning restore CS0618
        EffectConfigToken? token=null;
        if(effect is PropertyBasedEffect property) token=new PropertyBasedEffectConfigToken(property.CreatePropertyCollection());
        if(token is PropertyBasedEffectConfigToken propertyToken && (!args.Contains("--defaults") || args.Contains("--validate-dialog"))) {
            if(!Dialogs.Properties(effect.Name,propertyToken.Properties,ParameterLabels.For(effect,propertyToken.Properties),!args.Contains("--validate-dialog")))return 2;
        } else if(token is not PropertyBasedEffectConfigToken && effect.Options.Flags.HasFlag(EffectFlags.Configurable)) {
            // Also for --defaults: without a dialog there is no token, and such a plugin needs one.
            using var dialog=effect.CreateConfigDialog();
            if(dialog!=null) {
                dialog.Effect=effect;
                if(token!=null)dialog.EffectToken=token;
                if(!args.Contains("--defaults") && !args.Contains("--validate-dialog") && dialog.ShowDialog()!=DialogResult.OK)return 2;
                token=dialog.EffectToken;
            }
        }
        if(!input.HasPixels)return 0;
        using var sa=new RenderArgs(input.Src);using var da=new RenderArgs(input.Dst);
        effect.SetRenderInfo(token,da,sa);
        var scans=input.Region.GetRegionScansReadOnlyInt();
        effect.Render(token,da,sa,scans,0,scans.Length);
        return 0;
    }

    // A paint.net update can change what the helper was built against: name that, not only the member.
    static string Explain(Exception e)
    {
        var cause=e.GetBaseException();
        return cause is MissingMemberException or TypeLoadException
            ? $"Diese paint.net-Version passt nicht zum Host (getestet mit 5.1): {cause.Message}\n{e}"
            : e.ToString();
    }
}

// The PCFX packet: the active layer, colors, brush width and selection coverage.
internal sealed class EffectInput : IDisposable
{
    internal int W,H;
    internal ColorBgra Primary,Secondary;
    internal float Brush;
    internal Surface Src=null!,Dst=null!;
    internal PdnRegion Region=null!;
    internal bool HasPixels;
    internal System.Drawing.Rectangle Bounds=>Region.GetBoundsInt();

    internal static EffectInput Read(byte[] data,bool standIn)
    {
        var result=new EffectInput();
        // --validate-dialog constructs an editor without an image: one empty pixel stands in.
        if(standIn) {
            result.W=result.H=1;result.Brush=2;result.Src=new Surface(1,1);result.Dst=new Surface(1,1);
            result.Region=new PdnRegion();result.Region.MakeEmpty();result.Region.Union(new System.Drawing.Rectangle(0,0,1,1));
            return result;
        }
        result.HasPixels=true;
        using var input=new BinaryReader(new MemoryStream(data,false));
        if(input.ReadUInt32()!=0x58464350) throw new InvalidDataException("Invalid PCFX header");
        int w=input.ReadInt32(),h=input.ReadInt32();
        if(w<1 || h<1 || w>65535 || h>65535 || (long)w*h>100_000_000) throw new InvalidDataException("Image dimensions exceed host limit");
        result.W=w;result.H=h;
        result.Primary=ColorBgra.FromUInt32(input.ReadUInt32());
        result.Secondary=ColorBgra.FromUInt32(input.ReadUInt32());
        result.Brush=input.ReadSingle();
        result.Src=new Surface(w,h);
        result.Dst=new Surface(w,h);
        var row=new byte[checked(w*4)];
        for(int y=0;y<h;y++) {
            input.BaseStream.ReadExactly(row);
            unsafe {row.AsSpan().CopyTo(new Span<byte>(result.Src.GetRowPointer(y),row.Length));}
        }
        var mask=new byte[checked(w*h)];input.BaseStream.ReadExactly(mask);
        result.Region=new PdnRegion();result.Region.MakeEmpty();
        for(int y=0;y<h;y++) for(int x=0;x<w;) {
            while(x<w && mask[y*w+x]==0)x++;
            int start=x;while(x<w && mask[y*w+x]!=0)x++;
            if(start<x)result.Region.Union(new System.Drawing.Rectangle(start,y,x-start,1));
        }
        result.Dst.CopySurface(result.Src);
        return result;
    }

    // The constructor is marked obsolete with error for plugins; a host has to call it.
    internal EffectEnvironmentParameters Environment()=>
        (EffectEnvironmentParameters)Activator.CreateInstance(typeof(EffectEnvironmentParameters),Primary,Secondary,Brush,Region,Src)!;

    internal void WriteResult()
    {
        using var output=new BinaryWriter(Console.OpenStandardOutput());
        output.Write(0x58464350u);output.Write(W);output.Write(H);
        var row=new byte[checked(W*4)];
        for(int y=0;y<H;y++) {
            unsafe {new ReadOnlySpan<byte>(Dst.GetRowPointer(y),row.Length).CopyTo(row);}
            output.Write(row);
        }
    }

    public void Dispose() {Src?.Dispose();Dst?.Dispose();Region?.Dispose();}
}

internal static class Dialogs
{
    // A run without dialogs has no person to answer a plugin's error message.
    internal static bool Interactive=true;

    // show false only builds the editor: --validate-dialog checks a plugin's parameters that way.
    internal static bool Properties(string title,PropertyCollection properties,ControlInfo? labels,bool show)
    {
        Application.EnableVisualStyles();
        using var dialog=new Form {Text="PaintCraft – "+title,Width=580,Height=600,StartPosition=FormStartPosition.CenterScreen};
        var shown=new PluginProperties(properties,labels);
        var grid=new PropertyGrid {Dock=DockStyle.Fill,SelectedObject=shown,ToolbarVisible=false,PropertySort=PropertySort.NoSort};
        // --validate-dialog: each parameter must show its value and take the shown text back.
        if(!show)foreach(PropertyDescriptor p in shown.GetProperties()) {
            var text=p.Converter.ConvertToString(p.GetValue(shown));
            if(!p.IsReadOnly && p.Converter.CanConvertFrom(typeof(string)))p.SetValue(shown,p.Converter.ConvertFromString(text!));
            Console.Error.WriteLine($"parameter:{p.Name}={p.DisplayName} [{p.PropertyType.Name}] {text}");
        }
        var buttons=new FlowLayoutPanel {Dock=DockStyle.Bottom,Height=40,FlowDirection=FlowDirection.RightToLeft};
        var ok=new Button {Text="Anwenden",DialogResult=DialogResult.OK};
        var cancel=new Button {Text="Abbrechen",DialogResult=DialogResult.Cancel};
        buttons.Controls.Add(cancel);buttons.Controls.Add(ok);dialog.Controls.Add(grid);dialog.Controls.Add(buttons);
        dialog.AcceptButton=ok;dialog.CancelButton=cancel;
        grid.CreateControl();
        return !show || dialog.ShowDialog()==DialogResult.OK;
    }
}

// Paint.NET's IndirectUI is initialized by its application, not its public plugin API.
// Present typed PropertySystem values without relying on private initialization hooks.
sealed class PluginProperties(PropertyCollection values,ControlInfo? labels) : CustomTypeDescriptor
{
    // The plugin's order; a property it leaves out of its UI keeps its value, as in paint.net.
    readonly Lazy<PropertyDescriptor[]> shown=new(()=>{
        var names=new List<string>();
        void Walk(ControlInfo c) {
            if(c is PropertyControlInfo p)names.Add(p.Property.Name);
            foreach(var child in c.ChildControls)Walk(child);
        }
        if(labels!=null)Walk(labels);
        var order=names.Count>0 ? names.Select(n=>values.FirstOrDefault(p=>p.Name==n)).OfType<Property>() : values;
        // Tab pages and links are controls of paint.net's dialog, nothing to set here.
        return order.Where(p=>p.ValueType!=typeof(Uri) && p.ValueType.Name!="TabContainerState").Select(p=>(PropertyDescriptor)new PluginProperty(p,labels?.FindControlForPropertyName(p.Name))).ToArray();
    });
    public override PropertyDescriptorCollection GetProperties()=>GetProperties(null);
    public override PropertyDescriptorCollection GetProperties(Attribute[]? attributes)=>new(shown.Value);
    public override object GetPropertyOwner(PropertyDescriptor? pd)=>this;
}
sealed class PluginProperty(Property value,PropertyControlInfo? control) : PropertyDescriptor(value.Name,Array.Empty<Attribute>())
{
    string Text(string name)=>control?.ControlProperties.FirstOrDefault(p=>p.Name==name)?.Value as string ?? "";
    // Check boxes often carry their label as the description and no display name.
    public override string DisplayName=>Text("DisplayName").Trim() is {Length:>0} name ? name : Text("Description").Trim() is {Length:>0} text ? text : value.Name;
    public override string Description
    {
        get {
            var range=value.ValueType!=typeof(bool) && !IsColor && value.GetType().GetProperty("MinValue") is {} min && value.GetType().GetProperty("MaxValue") is {} max
                ? $"{min.GetValue(value)} bis {max.GetValue(value)}" : "";
            return Text("DisplayName").Length>0 && Text("Description").Length>0 ? (Text("Description")+" "+range).Trim() : range;
        }
    }
    public override Type ComponentType=>typeof(PluginProperties);
    public override Type PropertyType=>value.ValueType;
    public override bool IsReadOnly=>value.ReadOnly;
    public override bool CanResetValue(object component)=>!value.ReadOnly;
    public override object? GetValue(object? component)=>value.Value;
    public override void SetValue(object? component,object? newValue){value.Value=newValue!;OnValueChanged(component,EventArgs.Empty);}
    public override void ResetValue(object component)=>SetValue(component,value.DefaultValue);
    public override bool ShouldSerializeValue(object component)=>false;
    // Plugins store most colors as an int and let the color wheel show it.
    bool IsColor=>value.ValueType==typeof(int) && control?.ControlType.Value is PropertyControlType.ColorWheel;
    public override TypeConverter Converter=>value switch {
        StaticListChoiceProperty choices=>new ChoicesConverter(choices.ValueChoices,control),
        _ when IsColor=>new ColorConverter(),
        _ when value.ValueType==typeof(Pair<double,double>)=>new PairConverter(),
        _=>base.Converter
    };
    public override object? GetEditor(Type editorBaseType)=>IsColor && editorBaseType==typeof(UITypeEditor) ? new ColorEditor() : base.GetEditor(editorBaseType);
}
// #RRGGBB; what the upper byte means is the plugin's business, and an edit keeps it.
sealed class ColorConverter : TypeConverter
{
    public override bool CanConvertFrom(ITypeDescriptorContext? context,Type sourceType)=>sourceType==typeof(string);
    public override object? ConvertTo(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object? value,Type destinationType)=>
        destinationType==typeof(string) && value is int color ? "#"+(color&0xFFFFFF).ToString("X6") : base.ConvertTo(context,culture,value,destinationType);
    public override object? ConvertFrom(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object value)
    {
        int upper=context?.PropertyDescriptor?.GetValue(context.Instance) is int old ? old&unchecked((int)0xFF000000) : 0;
        return upper|Convert.ToInt32(value.ToString()!.Trim().TrimStart('#'),16)&0xFFFFFF;
    }
}
sealed class ColorEditor : UITypeEditor
{
    public override UITypeEditorEditStyle GetEditStyle(ITypeDescriptorContext? context)=>UITypeEditorEditStyle.Modal;
    public override object? EditValue(ITypeDescriptorContext? context,IServiceProvider provider,object? value)
    {
        int old=value is int color ? color : 0;
        using var dialog=new ColorDialog {FullOpen=true,Color=System.Drawing.Color.FromArgb(old|unchecked((int)0xFF000000))};
        return dialog.ShowDialog()==DialogResult.OK ? old&unchecked((int)0xFF000000)|dialog.Color.ToArgb()&0xFFFFFF : value;
    }
}
sealed class PairConverter : TypeConverter
{
    public override bool CanConvertFrom(ITypeDescriptorContext? context,Type sourceType)=>sourceType==typeof(string);
    public override object? ConvertTo(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object? value,Type destinationType)=>
        destinationType==typeof(string) && value is Pair<double,double> pair ? string.Create(culture,$"{pair.First}; {pair.Second}") : base.ConvertTo(context,culture,value,destinationType);
    public override object? ConvertFrom(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object value)
    {
        var parts=value.ToString()!.Split(';');
        if(parts.Length!=2)throw new FormatException("Zwei Werte mit ; getrennt, zum Beispiel 0,5; 0,25");
        return Pair.Create(double.Parse(parts[0],culture),double.Parse(parts[1],culture));
    }
}
sealed class ChoicesConverter(object[] choices,PropertyControlInfo? control) : TypeConverter
{
    string Label(object? choice)=>choice==null ? "" : control?.GetValueDisplayName(choice) ?? choice.ToString() ?? "";
    public override bool GetStandardValuesSupported(ITypeDescriptorContext? context)=>true;
    public override bool GetStandardValuesExclusive(ITypeDescriptorContext? context)=>true;
    public override StandardValuesCollection GetStandardValues(ITypeDescriptorContext? context)=>new(choices);
    public override bool CanConvertFrom(ITypeDescriptorContext? context,Type sourceType)=>sourceType==typeof(string);
    public override object? ConvertFrom(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object value)=>choices.FirstOrDefault(x=>Label(x)==value.ToString()) ?? base.ConvertFrom(context,culture,value);
    public override object? ConvertTo(ITypeDescriptorContext? context,System.Globalization.CultureInfo? culture,object? value,Type destinationType)=>destinationType==typeof(string) ? Label(value) : base.ConvertTo(context,culture,value,destinationType);
}
