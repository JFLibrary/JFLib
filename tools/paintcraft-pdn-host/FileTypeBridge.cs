using System.IO;
using PaintDotNet;

internal static class FileTypeBridge
{
    const uint Magic=0x4D494350; // PCIM: width, height, DPI x/y (16.16), original layer count, BGRA
    internal static int Run(string[] args,byte[] data)
    {
        var assembly=PluginCatalog.Load(args[0]);
        var key=args[1].Split('#');
        var instance=Activator.CreateInstance(assembly.GetType(key[0],true)!)!;
        var ft=instance is IFileTypeFactory factory ? factory.GetFileTypeInstances()[int.Parse(key[1])] : (FileType)instance;
        if(args.Contains("--decode")) {
            using var document=ft.Load(new MemoryStream(data,false));
            CheckSize(document.Width,document.Height);
            using var surface=new Surface(document.Width,document.Height);
            document.Flatten(surface);
            using var packet=new MemoryStream();using var writer=new BinaryWriter(packet);
            writer.Write(Magic);writer.Write(document.Width);writer.Write(document.Height);
            double factor=document.DpuUnit==MeasurementUnit.Centimeter?2.54:1;
            writer.Write(Dpi(document.DpuX*factor));writer.Write(Dpi(document.DpuY*factor));writer.Write(document.Layers.Count);
            WritePixels(writer,surface);
            packet.WriteTo(Console.OpenStandardOutput());
        } else {
            using var input=new BinaryReader(new MemoryStream(data,false));
            if(input.ReadUInt32()!=Magic)throw new InvalidDataException("Invalid PCIM packet");
            int w=input.ReadInt32(),h=input.ReadInt32();CheckSize(w,h);
            double dx=input.ReadUInt32()/65536.0,dy=input.ReadUInt32()/65536.0;input.ReadUInt32();
            if(data.Length!=24L+w*(long)h*4)throw new InvalidDataException("Invalid PCIM length");
            using var document=new Document(w,h) {DpuUnit=MeasurementUnit.Inch,DpuX=dx>0?dx:96,DpuY=dy>0?dy:96};
            var layer=new BitmapLayer(w,h);document.Layers.Add(layer);
            var row=new byte[w*4];
            for(int y=0;y<h;y++){input.BaseStream.ReadExactly(row);unsafe{row.CopyTo(new Span<byte>(layer.Surface.GetRowPointer(y),row.Length));}}
            using var scratch=new Surface(w,h);
            using var output=new MemoryStream();
            ft.Save(document,output,ft.CreateDefaultSaveConfigToken(),scratch,null!,false);
            if(output.Length>500_000_000)throw new InvalidDataException("Encoded file exceeds host limit");
            output.WriteTo(Console.OpenStandardOutput());
        }
        return 0;
    }
    static uint Dpi(double value)=>(uint)Math.Clamp(value*65536,0,uint.MaxValue);
    static void CheckSize(int w,int h){if(w<1||h<1||w>65535||h>65535||(long)w*h>100_000_000)throw new InvalidDataException("Image exceeds host limit");}
    static void WritePixels(BinaryWriter writer,Surface sf)
    {
        var row=new byte[sf.Width*4];
        for(int y=0;y<sf.Height;y++){unsafe{new ReadOnlySpan<byte>(sf.GetRowPointer(y),row.Length).CopyTo(row);}writer.Write(row);}
    }
}
